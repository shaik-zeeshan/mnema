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
fn weekly_multi_day_question_windows_from_the_previous_selected_run() {
    use super::super::schedule::{ScheduleCadence, ScheduleWeekday};
    let trigger = TriggerDefinition {
        condition: super::super::TriggerCondition::Schedule {
            cadence: ScheduleCadence::Weekly,
            time: "18:00".to_string(),
            weekday: None,
            weekdays: Some(vec![ScheduleWeekday::Monday, ScheduleWeekday::Wednesday]),
        },
        ..sample_daily()
    };
    // Fired Wednesday 2026-07-22 18:00 UTC — the previous selected occurrence
    // is Monday 18:00, so the window runs Monday 18:00 → now (not week start).
    let wednesday_18 = 1_784_505_600_000 + 2 * 86_400_000 + 18 * 3_600_000;
    let question =
        build_firing_question(&trigger, wednesday_18, wednesday_18 + 5 * 60_000, 0, None);
    assert!(question.contains("weekly on Monday, Wednesday at 18:00"));
    assert!(question.contains("since this trigger's previous scheduled run"));
    assert!(question.contains("2026-07-20 18:00 to 2026-07-22 18:05"));

    // Fired MONDAY (no earlier selected day this week): week-so-far window.
    let monday_18 = 1_784_505_600_000 + 18 * 3_600_000;
    let question = build_firing_question(&trigger, monday_18, monday_18 + 5 * 60_000, 0, None);
    assert!(question.contains("the week so far (Monday-start)"));
    assert!(question.contains("2026-07-20 00:00 to 2026-07-20 18:05"));
}

#[test]
fn running_registry_marks_keeps_earliest_start_and_clears() {
    assert_eq!(trigger_running_since_ms("reg-test"), None);
    mark_trigger_running("reg-test");
    let started = trigger_running_since_ms("reg-test").expect("marked");
    // Re-marking (Readiness Wait → run) keeps the earliest start.
    mark_trigger_running("reg-test");
    assert_eq!(trigger_running_since_ms("reg-test"), Some(started));
    clear_trigger_running("reg-test");
    assert_eq!(trigger_running_since_ms("reg-test"), None);
    // Clearing an unmarked id (a skip recorded without a mark) is a no-op.
    clear_trigger_running("reg-test-never-marked");
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
