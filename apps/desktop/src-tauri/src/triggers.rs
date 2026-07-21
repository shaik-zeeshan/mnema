//! Triggers — definitions + evaluator (issues #175/#176/#177, ADRs 0057/0058).
//!
//! A **Trigger** is Condition + Prompt + Delivery (docs/triggers/CONTEXT.md).
//! This module owns the definition shape and the evaluator worker; the firing →
//! sealed Ask AI run path (retries, ledger rows, delivery) lives in
//! [`run`](self::run). The Meeting Ends condition is event-driven and lives in
//! its own worker ([`meeting`](self::meeting), with the Readiness Wait in
//! [`readiness`](self::readiness)) — the schedule tick below skips it.
//!
//! - Definitions are CONFIG, not DB (ADR 0058): `triggers.json` in the app
//!   config dir, hand-edited until the #182 management UI. It is re-read on
//!   every evaluator tick, so edits hot-reload within one tick.
//! - A background evaluator worker (the user-context worker's poll-loop
//!   pattern) fires due Schedule occurrences: daily/weekly at a chosen local
//!   time, catch-up within the natural period (day/week) — including across a
//!   restart, via the per-trigger last-fired row in the encrypted DB
//!   (`app_infra::trigger_state`) — expired occurrences quietly missed. The
//!   existing `system_did_wake` notifier nudges the loop so a wake catches up
//!   immediately instead of waiting out the tick.
//! - Every firing decision is accountable (issue #176): one `trigger_firings`
//!   ledger row per firing (completed/skipped/failed with an honest reason),
//!   a persisted per-trigger **Cooldown** (default 10 min) enforced from the
//!   ledger so it survives restarts, and a run-time **Provider Gate** — a run
//!   never starts unconfigured; needs-provider is a trigger state (visible via
//!   [`list_triggers_status`]), not a run failure.
//! - **Delivery** is good-news-only: a macOS notification on a COMPLETED run
//!   only; skips and failures surface quietly as last-run status.
//!
//! The per-trigger firing state machine ([`firing_decision`], pure):
//! due occurrence → cooldown → provider gate → claim → run+retries → ledger
//! row → notify (completed only).
//!
//! ponytail: firings run inline on the evaluator loop (one at a time);
//! parallel firings can arrive once something actually needs them.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{Listener, Manager};

use crate::app_infra::{shutdown_aware_sleep, AppInfraState, BackgroundWorkersState};
use crate::user_context::worker::now_ms;

pub(crate) mod app_opened;
pub(crate) mod meeting;
pub(crate) mod meeting_browser;
pub(crate) mod meeting_worker;
pub(crate) mod readiness;
pub(crate) mod run;
pub(crate) mod schedule;

pub(crate) use run::take_recent_notification_conversation;
use schedule::{ScheduleCadence, ScheduleWeekday};

/// The trigger definitions file inside the app config dir (ADR 0058).
pub const TRIGGERS_FILE_NAME: &str = "triggers.json";

/// Evaluator poll interval. Coarse on purpose: a Schedule's resolution is one
/// minute, and catch-up correctness comes from the due-occurrence math, not the
/// tick rate.
const TRIGGERS_TICK: Duration = Duration::from_secs(30);

/// Default per-trigger Cooldown (docs/triggers/CONTEXT.md): a Trigger never
/// fires again within this window of its last firing, regardless of Condition.
const DEFAULT_COOLDOWN_MINUTES: u32 = 10;

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
    /// Per-trigger Cooldown override in minutes ("Advanced Options");
    /// [`DEFAULT_COOLDOWN_MINUTES`] when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_minutes: Option<u32>,
    #[serde(default = "default_version")]
    pub version: u32,
}

impl TriggerDefinition {
    /// The trigger's Cooldown window in milliseconds.
    pub fn cooldown_ms(&self) -> i64 {
        i64::from(self.cooldown_minutes.unwrap_or(DEFAULT_COOLDOWN_MINUTES)) * 60_000
    }
}

/// The Condition menu, tagged on `type`.
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
    /// `{"type":"meeting_ends"}` — fires when a Meeting (ADR 0057: an
    /// allowlisted conferencing app's mic hold) ends. Evaluated by the meeting
    /// detector worker ([`meeting`]), not the schedule tick.
    #[serde(rename = "meeting_ends", rename_all = "camelCase")]
    MeetingEnds {
        /// Per-trigger minimum meeting length in minutes ("Advanced Options");
        /// 5 when absent. Shorter mic holds never fire this trigger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_meeting_minutes: Option<u32>,
    },
    /// `{"type":"app_opened","bundleId":"com.figma.Desktop","appName":"Figma"}`
    /// — fires when the chosen app becomes frontmost after ≥ the away gap of
    /// not being frontmost (a fresh session). Evaluated by the app-opened
    /// worker ([`app_opened`]) fed from the NSWorkspace activation observer,
    /// not the schedule tick.
    #[serde(rename = "app_opened", rename_all = "camelCase")]
    AppOpened {
        bundle_id: String,
        /// Display name for the firing context and #182 UI — the definition
        /// carries it so firing never needs an installed-apps lookup.
        app_name: String,
        /// Per-trigger away gap in minutes ("Advanced Options"); 30 when
        /// absent. Shorter absences never fire this trigger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        away_gap_minutes: Option<u32>,
    },
}

/// The event-condition cooldown anchor (Meeting Ends, App Opened): an event
/// firing may write its ledger row well after claim time (the meeting path
/// waits up to 15 min on readiness; every path runs the multi-minute AI turn),
/// but the `trigger_state` claim cursor is written at claim time — so take the
/// newest of both, and a second event inside the cooldown of a still-in-flight
/// firing is suppressed.
pub(crate) fn event_cooldown_anchor_ms(
    ledger_ms: Option<i64>,
    claim_cursor_ms: Option<i64>,
) -> Option<i64> {
    ledger_ms.max(claim_cursor_ms)
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

// ── The firing decision (pure) ───────────────────────────────────────────────

/// What the evaluator should do about one trigger this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FiringDecision {
    /// No due occurrence this period.
    NotDue,
    /// Due, but inside the Cooldown window of the last ledger firing. A
    /// suppression, NOT a Skipped Run: no occurrence claim and no ledger row,
    /// so the occurrence stays due and fires once the window passes (still
    /// within its natural period).
    CooldownSuppressed,
    /// Due, but the Reasoning Engine is unconfigured (Provider Gate). Do NOT
    /// claim the occurrence and do NOT write a ledger row — unconfigured is a
    /// trigger state (surfaced by [`list_triggers_status`]), not a run
    /// failure. Configuring a provider later within the same period still
    /// catches up.
    NeedsProvider,
    /// Claim the occurrence and run the firing.
    Fire { occurrence_ms: i64 },
}

/// The per-trigger firing state machine, pure over its already-read inputs:
/// due occurrence → Cooldown (from the ledger's newest row, ANY outcome — so
/// it survives restarts) → Provider Gate → fire.
pub(crate) fn firing_decision(
    due_occurrence_ms: Option<i64>,
    last_firing_ms: Option<i64>,
    cooldown_ms: i64,
    provider_ready: bool,
    now_ms: i64,
) -> FiringDecision {
    let Some(occurrence_ms) = due_occurrence_ms else {
        return FiringDecision::NotDue;
    };
    if last_firing_ms.is_some_and(|fired| now_ms.saturating_sub(fired) < cooldown_ms) {
        return FiringDecision::CooldownSuppressed;
    }
    if !provider_ready {
        return FiringDecision::NeedsProvider;
    }
    FiringDecision::Fire { occurrence_ms }
}

// ── Evaluator worker ─────────────────────────────────────────────────────────

/// One evaluator pass: hot-reload `triggers.json`, then decide + fire every
/// enabled Schedule trigger whose current-period occurrence is due.
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
        // Meeting Ends is event-driven, evaluated by the meeting detector
        // worker — the schedule tick only handles Schedule conditions.
        let TriggerCondition::Schedule {
            cadence,
            ref time,
            weekday,
        } = trigger.condition
        else {
            continue;
        };
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
        let due = schedule::due_occurrence_ms(
            cadence,
            time_minutes,
            weekday,
            now,
            offset_minutes,
            last_fired,
        );
        if due.is_none() {
            continue;
        }

        // The reads the pure decision needs, gathered only once something is
        // due: the Cooldown anchor (newest ledger row, any outcome) and the
        // Provider Gate.
        let last_firing_ms = infra
            .trigger_firings()
            .last_firing(&trigger.id)
            .await
            .ok()
            .flatten()
            .map(|firing| firing.fired_at_ms);
        let provider_ready = crate::ask_ai::ensure_ask_ai_access_ready(app_handle)
            .await
            .is_ok();

        match firing_decision(due, last_firing_ms, trigger.cooldown_ms(), provider_ready, now) {
            FiringDecision::NotDue => continue,
            FiringDecision::CooldownSuppressed => {
                tauri_plugin_log::log::debug!(
                    "triggers: trigger '{}' is due but cooling down; will retry",
                    trigger.id
                );
                continue;
            }
            FiringDecision::NeedsProvider => {
                tauri_plugin_log::log::debug!(
                    "triggers: trigger '{}' is due but the engine is not configured; will retry",
                    trigger.id
                );
                continue;
            }
            FiringDecision::Fire { occurrence_ms } => {
                // Durably claim the occurrence BEFORE running: retries happen
                // WITHIN this one firing (run.rs), never as a re-fire — a
                // crash mid-run quietly misses the occurrence rather than
                // re-billing the model every tick.
                if let Err(error) = infra.trigger_state().set_last_fired_ms(&trigger.id, now).await
                {
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
                // ponytail: firings run inline, one at a time.
                run::run_trigger_fire(
                    app_handle,
                    infra,
                    &trigger,
                    occurrence_ms,
                    offset_minutes,
                    None,
                )
                .await;
            }
        }
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

// ── Status (the #182 management-UI seam) ─────────────────────────────────────

/// The newest ledger row for a trigger, as last-run status.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerLastFiring {
    pub fired_at_ms: i64,
    /// `completed` / `skipped` / `failed`.
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

/// One trigger's runtime status: definition basics, the Provider Gate state,
/// and its last firing from the ledger.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerStatus {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Provider Gate (docs/triggers/CONTEXT.md): `true` when the Reasoning
    /// Engine is unconfigured — the trigger is visibly disabled ("needs an AI
    /// provider") and the evaluator will not start runs.
    pub needs_provider: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_firing: Option<TriggerLastFiring>,
}

/// List every defined trigger with its runtime status. Derived fresh on every
/// call: `needs_provider` runs the SAME gate the evaluator does, so the UI and
/// the evaluator can never disagree about "needs a provider".
#[tauri::command]
pub async fn list_triggers_status(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppInfraState>,
) -> Result<Vec<TriggerStatus>, String> {
    let infra = Arc::clone(&*state);
    let needs_provider = crate::ask_ai::ensure_ask_ai_access_ready(&app_handle)
        .await
        .is_err();
    let mut statuses = Vec::new();
    for trigger in load_triggers(&app_handle) {
        let last_firing = infra
            .trigger_firings()
            .last_firing(&trigger.id)
            .await
            .map_err(|error| {
                format!(
                    "failed to read the firing ledger for trigger '{}': {error}",
                    trigger.id
                )
            })?
            .map(|firing| TriggerLastFiring {
                fired_at_ms: firing.fired_at_ms,
                outcome: firing.outcome.as_str().to_string(),
                reason: firing.reason,
                conversation_id: firing.conversation_id,
            });
        statuses.push(TriggerStatus {
            id: trigger.id,
            name: trigger.name,
            enabled: trigger.enabled,
            needs_provider,
            last_firing,
        });
    }
    Ok(statuses)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn sample_daily() -> TriggerDefinition {
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
            cooldown_minutes: None,
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

        // Weekly carries its weekday tag; a Cooldown override rides as
        // `cooldownMinutes` ("Advanced Options").
        let weekly = TriggerDefinition {
            condition: TriggerCondition::Schedule {
                cadence: ScheduleCadence::Weekly,
                time: "09:00".to_string(),
                weekday: Some(ScheduleWeekday::Friday),
            },
            cooldown_minutes: Some(30),
            ..sample_daily()
        };
        let value = serde_json::to_value(&weekly).unwrap();
        assert_eq!(
            value["condition"],
            json!({ "type": "schedule", "cadence": "weekly", "time": "09:00", "weekday": "friday" })
        );
        assert_eq!(value["cooldownMinutes"], json!(30));
        let round_tripped: TriggerDefinition = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, weekly);
    }

    #[test]
    fn meeting_ends_condition_serde_round_trips_and_pins_the_wire_shape() {
        // Minimal form: `{"type":"meeting_ends"}` (issue #177). The absent
        // floor stays absent on the wire (shareable JSON stays minimal).
        let trigger = TriggerDefinition {
            condition: TriggerCondition::MeetingEnds {
                min_meeting_minutes: None,
            },
            ..sample_daily()
        };
        let value = serde_json::to_value(&trigger).unwrap();
        assert_eq!(value["condition"], json!({ "type": "meeting_ends" }));
        let round_tripped: TriggerDefinition = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, trigger);

        // The per-trigger floor rides as `minMeetingMinutes` ("Advanced
        // Options").
        let with_floor = TriggerDefinition {
            condition: TriggerCondition::MeetingEnds {
                min_meeting_minutes: Some(10),
            },
            ..sample_daily()
        };
        let value = serde_json::to_value(&with_floor).unwrap();
        assert_eq!(
            value["condition"],
            json!({ "type": "meeting_ends", "minMeetingMinutes": 10 })
        );
        let round_tripped: TriggerDefinition = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, with_floor);
    }

    #[test]
    fn app_opened_condition_serde_round_trips_and_pins_the_wire_shape() {
        // Minimal form (issue #178): bundle id + display name, away gap absent
        // on the wire (shareable JSON stays minimal).
        let trigger = TriggerDefinition {
            condition: TriggerCondition::AppOpened {
                bundle_id: "com.figma.Desktop".to_string(),
                app_name: "Figma".to_string(),
                away_gap_minutes: None,
            },
            ..sample_daily()
        };
        let value = serde_json::to_value(&trigger).unwrap();
        assert_eq!(
            value["condition"],
            json!({ "type": "app_opened", "bundleId": "com.figma.Desktop", "appName": "Figma" })
        );
        let round_tripped: TriggerDefinition = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, trigger);

        // The per-trigger gap rides as `awayGapMinutes` ("Advanced Options").
        let with_gap = TriggerDefinition {
            condition: TriggerCondition::AppOpened {
                bundle_id: "com.figma.Desktop".to_string(),
                app_name: "Figma".to_string(),
                away_gap_minutes: Some(120),
            },
            ..sample_daily()
        };
        let value = serde_json::to_value(&with_gap).unwrap();
        assert_eq!(
            value["condition"],
            json!({
                "type": "app_opened",
                "bundleId": "com.figma.Desktop",
                "appName": "Figma",
                "awayGapMinutes": 120
            })
        );
        let round_tripped: TriggerDefinition = serde_json::from_value(value).unwrap();
        assert_eq!(round_tripped, with_gap);
    }

    #[test]
    fn event_cooldown_anchor_is_the_newest_of_ledger_and_claim_cursor() {
        assert_eq!(event_cooldown_anchor_ms(None, None), None);
        assert_eq!(event_cooldown_anchor_ms(Some(5), None), Some(5));
        assert_eq!(event_cooldown_anchor_ms(None, Some(7)), Some(7));
        assert_eq!(event_cooldown_anchor_ms(Some(5), Some(7)), Some(7));
        assert_eq!(event_cooldown_anchor_ms(Some(9), Some(7)), Some(9));
    }

    #[test]
    fn trigger_definition_defaults_enabled_version_and_cooldown() {
        // A minimal hand-authored file omits `enabled`/`version`/`cooldownMinutes`.
        let parsed: TriggerDefinition = serde_json::from_value(json!({
            "id": "t",
            "name": "T",
            "condition": { "type": "schedule", "cadence": "daily", "time": "08:00" },
            "prompt": "p"
        }))
        .unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.cooldown_minutes, None);
        assert_eq!(parsed.cooldown_ms(), 10 * 60_000);

        // The per-trigger override wins.
        let overridden = TriggerDefinition {
            cooldown_minutes: Some(30),
            ..sample_daily()
        };
        assert_eq!(overridden.cooldown_ms(), 30 * 60_000);
    }

    // ── firing_decision: due → cooldown → provider gate → fire ───────────────

    const MIN_MS: i64 = 60_000;

    #[test]
    fn decision_is_not_due_without_an_occurrence() {
        assert_eq!(
            firing_decision(None, None, 10 * MIN_MS, true, 1_000_000),
            FiringDecision::NotDue
        );
    }

    #[test]
    fn decision_cooldown_suppresses_a_second_firing_within_the_window() {
        let now = 100 * MIN_MS;
        // The last ledger firing (ANY outcome) 5 min ago holds a 10-min
        // cooldown — the ledger is persisted, so this is exactly the state a
        // fresh process reads after a restart.
        assert_eq!(
            firing_decision(Some(now), Some(now - 5 * MIN_MS), 10 * MIN_MS, true, now),
            FiringDecision::CooldownSuppressed
        );
        // At the window's edge (exactly 10 min elapsed) it fires again.
        assert_eq!(
            firing_decision(Some(now), Some(now - 10 * MIN_MS), 10 * MIN_MS, true, now),
            FiringDecision::Fire { occurrence_ms: now }
        );
        // No prior firing → no cooldown.
        assert_eq!(
            firing_decision(Some(now), None, 10 * MIN_MS, true, now),
            FiringDecision::Fire { occurrence_ms: now }
        );
    }

    #[test]
    fn decision_provider_gate_holds_the_occurrence_instead_of_failing() {
        let now = 100 * MIN_MS;
        // Provider gone → NeedsProvider: the evaluator claims the occurrence
        // and writes ledger rows ONLY on `Fire`, so the occurrence is not
        // burned and no `failed` row appears; the same call next tick (after
        // the provider returns) fires.
        assert_eq!(
            firing_decision(Some(now), None, 10 * MIN_MS, false, now),
            FiringDecision::NeedsProvider
        );
        assert_eq!(
            firing_decision(Some(now), None, 10 * MIN_MS, true, now),
            FiringDecision::Fire { occurrence_ms: now }
        );
        // Cooldown outranks the gate (nothing would fire either way).
        assert_eq!(
            firing_decision(Some(now), Some(now - MIN_MS), 10 * MIN_MS, false, now),
            FiringDecision::CooldownSuppressed
        );
    }
}
