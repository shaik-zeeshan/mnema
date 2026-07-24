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
            weekdays: None,
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

    // Weekly carries its weekday SET; a Cooldown override rides as
    // `cooldownMinutes` ("Advanced Options").
    let weekly = TriggerDefinition {
        condition: TriggerCondition::Schedule {
            cadence: ScheduleCadence::Weekly,
            time: "09:00".to_string(),
            weekday: None,
            weekdays: Some(vec![ScheduleWeekday::Monday, ScheduleWeekday::Friday]),
        },
        cooldown_minutes: Some(30),
        ..sample_daily()
    };
    let value = serde_json::to_value(&weekly).unwrap();
    assert_eq!(
        value["condition"],
        json!({
            "type": "schedule",
            "cadence": "weekly",
            "time": "09:00",
            "weekdays": ["monday", "friday"]
        })
    );
    assert_eq!(value["cooldownMinutes"], json!(30));
    let round_tripped: TriggerDefinition = serde_json::from_value(value).unwrap();
    assert_eq!(round_tripped, weekly);
}

#[test]
fn legacy_single_weekday_schedule_normalizes_into_the_weekday_set() {
    // A pre-multi-day file/shared payload: `"weekday":"friday"`. It still
    // parses, and `normalized()` (applied at every read seam) folds it into
    // `weekdays` so the evaluator and UI only ever see the set form.
    let parsed: TriggerDefinition = serde_json::from_value(json!({
        "id": "t",
        "name": "T",
        "condition": {
            "type": "schedule", "cadence": "weekly", "time": "09:00", "weekday": "friday"
        },
        "prompt": "p"
    }))
    .unwrap();
    let normalized = parsed.normalized();
    assert_eq!(
        normalized.condition,
        TriggerCondition::Schedule {
            cadence: ScheduleCadence::Weekly,
            time: "09:00".to_string(),
            weekday: None,
            weekdays: Some(vec![ScheduleWeekday::Friday]),
        }
    );
    assert_eq!(
        normalized.condition.schedule_weekdays(),
        &[ScheduleWeekday::Friday]
    );
    // Re-serialized, the legacy key is gone — only the set form remains.
    let value = serde_json::to_value(&normalized).unwrap();
    assert_eq!(
        value["condition"],
        json!({
            "type": "schedule", "cadence": "weekly", "time": "09:00",
            "weekdays": ["friday"]
        })
    );
    // When BOTH forms are present, the set wins and the legacy key drops.
    let both: TriggerDefinition = serde_json::from_value(json!({
        "id": "t",
        "name": "T",
        "condition": {
            "type": "schedule", "cadence": "weekly", "time": "09:00",
            "weekday": "monday", "weekdays": ["friday"]
        },
        "prompt": "p"
    }))
    .unwrap();
    assert_eq!(
        both.normalized().condition.schedule_weekdays(),
        &[ScheduleWeekday::Friday]
    );
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
