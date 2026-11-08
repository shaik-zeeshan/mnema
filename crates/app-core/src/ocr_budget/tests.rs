use super::*;

fn admission_signals(
    first_candidate_in_scope: bool,
    context_changed: bool,
    low_queue_pressure: bool,
    representative_due: bool,
    high_queue_pressure: bool,
) -> ::app_infra::OcrAdmissionSignals {
    ::app_infra::OcrAdmissionSignals {
        first_candidate_in_scope,
        context_changed,
        low_queue_pressure,
        representative_due,
        high_queue_pressure,
        fingerprint_novel_in_scope: false,
        novelty_admission_available: false,
    }
}

#[test]
fn active_recording_skips_recent_low_pressure_frames() {
    let signals = admission_signals(false, false, true, false, false);

    let decision = ocr_admission_decision_for_signals(&signals, 0, true);

    assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Skipped);
    assert_eq!(
        decision.reason,
        ::app_infra::OcrAdmissionReason::SkippedLowOcrValue
    );
}

#[test]
fn active_recording_admits_due_representative_frames() {
    let signals = admission_signals(false, false, true, true, false);

    let decision = ocr_admission_decision_for_signals(&signals, 0, true);

    assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Admitted);
    assert_eq!(
        decision.reason,
        ::app_infra::OcrAdmissionReason::AdmittedRepresentative
    );
}

#[test]
fn active_recording_respects_high_queue_pressure_for_representatives() {
    let signals = admission_signals(false, false, false, true, true);

    let decision = ocr_admission_decision_for_signals(&signals, HIGH_PRESSURE_THRESHOLD, true);

    assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Skipped);
    assert_eq!(
        decision.reason,
        ::app_infra::OcrAdmissionReason::SkippedLowOcrValue
    );
}

#[test]
fn active_recording_admits_novel_frame_when_novelty_available() {
    let signals = ::app_infra::OcrAdmissionSignals {
        fingerprint_novel_in_scope: true,
        novelty_admission_available: true,
        ..admission_signals(false, false, true, false, false)
    };

    let decision = ocr_admission_decision_for_signals(&signals, 0, true);

    assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Admitted);
    assert_eq!(
        decision.reason,
        ::app_infra::OcrAdmissionReason::AdmittedVisualNovelty
    );
}

#[test]
fn active_recording_skips_novel_frame_when_novelty_unavailable() {
    // Rate floor not satisfied or in a continuous-novelty burst.
    let signals = ::app_infra::OcrAdmissionSignals {
        fingerprint_novel_in_scope: true,
        novelty_admission_available: false,
        ..admission_signals(false, false, true, false, false)
    };

    let decision = ocr_admission_decision_for_signals(&signals, 0, true);

    assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Skipped);
    assert_eq!(
        decision.reason,
        ::app_infra::OcrAdmissionReason::SkippedLowOcrValue
    );
}

#[test]
fn active_recording_skips_novel_frame_under_high_queue_pressure() {
    let signals = ::app_infra::OcrAdmissionSignals {
        fingerprint_novel_in_scope: true,
        novelty_admission_available: true,
        ..admission_signals(false, false, false, false, true)
    };

    let decision = ocr_admission_decision_for_signals(&signals, HIGH_PRESSURE_THRESHOLD, true);

    assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Skipped);
    assert_eq!(
        decision.reason,
        ::app_infra::OcrAdmissionReason::SkippedLowOcrValue
    );
}

#[test]
fn active_recording_prefers_representative_over_novelty_when_due() {
    // A due representative outranks the novelty path even if both qualify.
    let signals = ::app_infra::OcrAdmissionSignals {
        fingerprint_novel_in_scope: true,
        novelty_admission_available: true,
        ..admission_signals(false, false, true, true, false)
    };

    let decision = ocr_admission_decision_for_signals(&signals, 0, true);

    assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Admitted);
    assert_eq!(
        decision.reason,
        ::app_infra::OcrAdmissionReason::AdmittedRepresentative
    );
}

#[test]
fn active_recording_skips_non_novel_frame_even_when_novelty_available() {
    let signals = ::app_infra::OcrAdmissionSignals {
        fingerprint_novel_in_scope: false,
        novelty_admission_available: true,
        ..admission_signals(false, false, true, false, false)
    };

    let decision = ocr_admission_decision_for_signals(&signals, 0, true);

    assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Skipped);
    assert_eq!(
        decision.reason,
        ::app_infra::OcrAdmissionReason::SkippedLowOcrValue
    );
}

#[test]
fn catch_up_still_admits_low_pressure_frames_when_not_recording() {
    let signals = admission_signals(false, false, true, false, false);

    let decision = ocr_admission_decision_for_signals(&signals, 0, false);

    assert_eq!(decision.outcome, ::app_infra::OcrAdmissionOutcome::Admitted);
    assert_eq!(
        decision.reason,
        ::app_infra::OcrAdmissionReason::AdmittedLowPressure
    );
}

#[test]
fn representative_window_detects_recent_admission() {
    let base_dir = PathBuf::from("/tmp/ocr-budget-test-representative");
    reset_for_base_dir(&base_dir);
    let key = AdmissionScopeKey {
        session_id: "session-a".to_string(),
        workspace_prefix: None,
    };
    with_state(&base_dir, |state| {
        state.admission_scopes.insert(
            key.clone(),
            AdmissionScopeState {
                seen_candidate: true,
                last_admitted_at: parse_rfc3339("2026-04-12T10:00:00Z"),
                ..Default::default()
            },
        );
    });

    let recent = with_state(&base_dir, |state| {
        let captured_at = parse_rfc3339("2026-04-12T10:00:14Z").unwrap();
        state
            .admission_scopes
            .get(&key)
            .and_then(|scope| scope.last_admitted_at)
            .map(|last| (captured_at - last).whole_seconds() <= REPRESENTATIVE_SECONDS)
            .unwrap_or(false)
    });
    let stale = with_state(&base_dir, |state| {
        let captured_at = parse_rfc3339("2026-04-12T10:00:16Z").unwrap();
        state
            .admission_scopes
            .get(&key)
            .and_then(|scope| scope.last_admitted_at)
            .map(|last| (captured_at - last).whole_seconds() <= REPRESENTATIVE_SECONDS)
            .unwrap_or(false)
    });

    assert!(recent);
    assert!(!stale);
}

#[test]
fn session_stop_clears_admission_memory() {
    let base_dir = PathBuf::from("/tmp/ocr-budget-test-clear");
    reset_for_base_dir(&base_dir);
    with_state(&base_dir, |state| {
        state.admission_scopes.insert(
            AdmissionScopeKey {
                session_id: "stopped".to_string(),
                workspace_prefix: None,
            },
            AdmissionScopeState {
                seen_candidate: true,
                last_admitted_at: parse_rfc3339("2026-04-12T10:00:00Z"),
                ..Default::default()
            },
        );
        state.admission_scopes.insert(
            AdmissionScopeKey {
                session_id: "active".to_string(),
                workspace_prefix: None,
            },
            AdmissionScopeState {
                seen_candidate: true,
                last_admitted_at: parse_rfc3339("2026-04-12T10:00:00Z"),
                ..Default::default()
            },
        );
    });

    clear_sessions_for_base_dir(&base_dir, &["stopped".to_string()]);

    with_state(&base_dir, |state| {
        assert!(!state
            .admission_scopes
            .keys()
            .any(|key| key.session_id == "stopped"));
        assert!(state
            .admission_scopes
            .keys()
            .any(|key| key.session_id == "active"));
    });
}

#[test]
fn fingerprint_is_novel_detects_first_occurrence_and_repeat() {
    let mut ring = VecDeque::new();
    assert!(fingerprint_is_novel(&ring, Some(7)));
    remember_fingerprint(&mut ring, 7);
    assert!(!fingerprint_is_novel(&ring, Some(7)));
    assert!(fingerprint_is_novel(&ring, Some(8)));
}

#[test]
fn fingerprint_without_equivalence_is_not_novel() {
    let ring = VecDeque::new();
    assert!(!fingerprint_is_novel(&ring, None));
}

#[test]
fn fingerprint_ring_evicts_oldest_and_re_novels_it() {
    let mut ring = VecDeque::new();
    for value in 0..(NOVELTY_RECENT_FINGERPRINT_CAPACITY as u64) {
        remember_fingerprint(&mut ring, value);
    }
    assert_eq!(ring.len(), NOVELTY_RECENT_FINGERPRINT_CAPACITY);
    // The oldest fingerprint (0) is still remembered until one more pushes it out.
    assert!(!fingerprint_is_novel(&ring, Some(0)));
    remember_fingerprint(&mut ring, u64::MAX);
    assert_eq!(ring.len(), NOVELTY_RECENT_FINGERPRINT_CAPACITY);
    // After eviction the oldest fingerprint reads as novel again.
    assert!(fingerprint_is_novel(&ring, Some(0)));
    assert!(!fingerprint_is_novel(&ring, Some(1)));
}

#[test]
fn novelty_rate_floor_respects_minimum_interval() {
    let last = parse_rfc3339("2026-04-12T10:00:00Z");
    // No prior novelty admission: floor is clear regardless of capture time.
    assert!(novelty_rate_floor_satisfied(
        parse_rfc3339("2026-04-12T10:00:00Z"),
        None
    ));
    // Below the interval: denied.
    assert!(!novelty_rate_floor_satisfied(
        parse_rfc3339("2026-04-12T10:00:01Z"),
        last
    ));
    // Exactly at the interval: satisfied.
    assert!(novelty_rate_floor_satisfied(
        parse_rfc3339("2026-04-12T10:00:02Z"),
        last
    ));
    // Unparseable capture time with a prior admission: denied.
    assert!(!novelty_rate_floor_satisfied(None, last));
}

#[test]
fn continuous_novelty_burst_trips_at_threshold() {
    assert!(!in_continuous_novelty_burst(
        NOVELTY_SUSTAINED_RUN_SUPPRESS - 1
    ));
    assert!(in_continuous_novelty_burst(NOVELTY_SUSTAINED_RUN_SUPPRESS));
    assert!(in_continuous_novelty_burst(
        NOVELTY_SUSTAINED_RUN_SUPPRESS + 5
    ));
}

/// Capture time `base_seconds + offset` (UTC) for building temporally
/// adjacent or spaced novel frames in novelty-memory tests.
fn captured_at_seconds(offset: i64) -> Option<OffsetDateTime> {
    Some(OffsetDateTime::from_unix_timestamp(1_700_000_000 + offset).unwrap())
}

fn admitted_novelty() -> (
    ::app_infra::OcrAdmissionOutcome,
    ::app_infra::OcrAdmissionReason,
) {
    (
        ::app_infra::OcrAdmissionOutcome::Admitted,
        ::app_infra::OcrAdmissionReason::AdmittedVisualNovelty,
    )
}

fn skipped_low_value() -> (
    ::app_infra::OcrAdmissionOutcome,
    ::app_infra::OcrAdmissionReason,
) {
    (
        ::app_infra::OcrAdmissionOutcome::Skipped,
        ::app_infra::OcrAdmissionReason::SkippedLowOcrValue,
    )
}

fn skipped_equivalent() -> (
    ::app_infra::OcrAdmissionOutcome,
    ::app_infra::OcrAdmissionReason,
) {
    (
        ::app_infra::OcrAdmissionOutcome::Skipped,
        ::app_infra::OcrAdmissionReason::SkippedEquivalentFrame,
    )
}

#[test]
fn temporally_continuous_novel_frames_grow_run_and_trip_burst() {
    let mut scope = AdmissionScopeState::default();
    let (outcome, reason) = admitted_novelty();
    // Distinct novel frames arriving one second apart (within the 2s gap).
    for step in 0..(NOVELTY_SUSTAINED_RUN_SUPPRESS as i64) {
        record_novelty_memory(
            &mut scope,
            true,
            Some(step as u64),
            outcome,
            reason,
            captured_at_seconds(step),
        );
    }
    assert_eq!(scope.consecutive_novel_run, NOVELTY_SUSTAINED_RUN_SUPPRESS);
    assert!(in_continuous_novelty_burst(scope.consecutive_novel_run));
}

#[test]
fn non_novel_frame_resets_run() {
    let mut scope = AdmissionScopeState {
        consecutive_novel_run: NOVELTY_SUSTAINED_RUN_SUPPRESS,
        last_novel_frame_at: captured_at_seconds(5),
        ..Default::default()
    };
    let (outcome, reason) = skipped_low_value();
    // A non-novel frame (decide-time not novel) resets the run to 0.
    record_novelty_memory(
        &mut scope,
        false,
        Some(99),
        outcome,
        reason,
        captured_at_seconds(6),
    );
    assert_eq!(scope.consecutive_novel_run, 0);
    assert!(!in_continuous_novelty_burst(scope.consecutive_novel_run));
}

#[test]
fn rate_floor_skipped_novel_frame_is_not_remembered_and_stays_novel() {
    // Finding #1: a NOVEL frame skipped for low OCR value (rate floor /
    // queue pressure) must NOT be remembered, so a later identical frame is
    // still novel and can be admitted once the floor clears.
    let mut scope = AdmissionScopeState::default();
    let (outcome, reason) = skipped_low_value();
    record_novelty_memory(
        &mut scope,
        true,
        Some(42),
        outcome,
        reason,
        captured_at_seconds(0),
    );
    // Fingerprint was not remembered.
    assert!(fingerprint_is_novel(&scope.recent_fingerprints, Some(42)));
    // A skipped frame is not novel-for-coverage but the decide-time novel
    // signal still moves the run; here a single skipped frame leaves run at
    // 1 (no prior novel frame to be continuous with).
    assert_eq!(scope.consecutive_novel_run, 1);

    // Later, the same screen is admitted via novelty: now it is covered.
    let (outcome, reason) = admitted_novelty();
    record_novelty_memory(
        &mut scope,
        true,
        Some(42),
        outcome,
        reason,
        captured_at_seconds(3),
    );
    assert!(!fingerprint_is_novel(&scope.recent_fingerprints, Some(42)));
}

#[test]
fn skipped_equivalent_resets_run_and_is_remembered() {
    // Finding #5: a SkippedEquivalentFrame is a confirmed duplicate. Even
    // though the stale decide-time signal says novel, it must reset the run
    // (not count as novel) yet its fingerprint IS covered, so remember it.
    let mut scope = AdmissionScopeState {
        consecutive_novel_run: 4,
        last_novel_frame_at: captured_at_seconds(0),
        ..Default::default()
    };
    let (outcome, reason) = skipped_equivalent();
    record_novelty_memory(
        &mut scope,
        true, // stale decide-time novelty
        Some(7),
        outcome,
        reason,
        captured_at_seconds(1),
    );
    // Confirmed duplicate resets the run.
    assert_eq!(scope.consecutive_novel_run, 0);
    // But its content is covered by reuse, so it is remembered.
    assert!(!fingerprint_is_novel(&scope.recent_fingerprints, Some(7)));
}

#[test]
fn temporal_gap_un_latches_burst_and_restarts_run() {
    // Finding #3: a continuous run of novel frames trips the burst, but a
    // large gap before the next novel frame restarts the run (un-latch).
    let mut scope = AdmissionScopeState::default();
    let (outcome, reason) = admitted_novelty();
    for step in 0..(NOVELTY_SUSTAINED_RUN_SUPPRESS as i64) {
        record_novelty_memory(
            &mut scope,
            true,
            Some(step as u64),
            outcome,
            reason,
            captured_at_seconds(step),
        );
    }
    assert!(in_continuous_novelty_burst(scope.consecutive_novel_run));

    // A novel frame far in the future (well past the continuous gap)
    // restarts the run at 1 rather than extending the burst.
    let far = (NOVELTY_SUSTAINED_RUN_SUPPRESS as i64) + 100;
    record_novelty_memory(
        &mut scope,
        true,
        Some(9999),
        outcome,
        reason,
        captured_at_seconds(far),
    );
    assert_eq!(scope.consecutive_novel_run, 1);
    assert!(!in_continuous_novelty_burst(scope.consecutive_novel_run));
}

#[test]
fn sub_second_gap_beyond_threshold_is_not_continuous() {
    // A gap of 2.9s exceeds the 2s continuous-gap threshold, so the frames
    // are NOT part of the same continuous-novelty burst. The earlier
    // `.whole_seconds()` comparison truncated 2.9 -> 2 and wrongly counted
    // it as continuous; the gap must be compared with sub-second precision.
    let previous = captured_at_seconds(0);
    let current = previous.map(|t| t + time::Duration::milliseconds(2_900));
    assert!(!novel_frame_is_temporally_continuous(previous, current));
}

#[test]
fn gap_at_exact_threshold_is_continuous() {
    // Exactly NOVELTY_CONTINUOUS_GAP_SECONDS apart is still continuous
    // (frames arriving "at least this often" build the burst).
    let previous = captured_at_seconds(0);
    let current = previous.map(|t| t + time::Duration::seconds(NOVELTY_CONTINUOUS_GAP_SECONDS));
    assert!(novel_frame_is_temporally_continuous(previous, current));
}

#[test]
fn regressing_timestamp_is_not_continuous() {
    // A current capture time earlier than the previous novel frame (clock
    // adjustment / out-of-order ingestion) yields a negative gap. That is
    // not a high-frequency novel burst, so it must NOT count as continuous
    // and should restart the run rather than extend it.
    let previous = captured_at_seconds(10);
    let current = captured_at_seconds(9);
    assert!(!novel_frame_is_temporally_continuous(previous, current));
}

#[test]
fn any_admitted_reason_advances_novelty_rate_floor_anchor() {
    // Finding #7: any Admitted frame (representative, initial, context,
    // low-pressure, novelty) advances last_novelty_admitted_at.
    let representative = (
        ::app_infra::OcrAdmissionOutcome::Admitted,
        ::app_infra::OcrAdmissionReason::AdmittedRepresentative,
    );
    let mut scope = AdmissionScopeState::default();
    record_novelty_memory(
        &mut scope,
        true,
        Some(1),
        representative.0,
        representative.1,
        captured_at_seconds(5),
    );
    assert_eq!(scope.last_novelty_admitted_at, captured_at_seconds(5));

    // A novelty admission also advances it.
    let (outcome, reason) = admitted_novelty();
    record_novelty_memory(
        &mut scope,
        true,
        Some(2),
        outcome,
        reason,
        captured_at_seconds(9),
    );
    assert_eq!(scope.last_novelty_admitted_at, captured_at_seconds(9));
}

#[test]
fn skipped_frame_does_not_advance_novelty_rate_floor_anchor() {
    let mut scope = AdmissionScopeState {
        last_novelty_admitted_at: captured_at_seconds(5),
        ..Default::default()
    };
    let (outcome, reason) = skipped_low_value();
    record_novelty_memory(
        &mut scope,
        true,
        Some(1),
        outcome,
        reason,
        captured_at_seconds(9),
    );
    // A skipped frame is not a real read of current content; anchor stays.
    assert_eq!(scope.last_novelty_admitted_at, captured_at_seconds(5));
}

#[test]
fn none_capture_time_never_overwrites_novelty_rate_floor_anchor() {
    // Finding #6: a None capture time must never be stamped into
    // last_novelty_admitted_at (which would permanently disable the floor).
    let mut scope = AdmissionScopeState {
        last_novelty_admitted_at: captured_at_seconds(5),
        ..Default::default()
    };
    let (outcome, reason) = admitted_novelty();
    record_novelty_memory(&mut scope, true, Some(1), outcome, reason, None);
    // Prior parseable anchor is preserved, not clobbered by None.
    assert_eq!(scope.last_novelty_admitted_at, captured_at_seconds(5));
}

/// The governor's whole job is a duty cycle: OCR must not occupy more than
/// `1 / (1 + multiplier)` of the wall clock, because that fraction is what the
/// machine feels as heat. Asserted as the duty cycle rather than the raw
/// cooldown so the intent survives a retune of the multipliers.
#[test]
fn cooldown_holds_the_duty_cycle_in_both_bands() {
    let duty = |run_ms: u64, recording: bool| {
        let cooldown = cooldown_duration(run_ms, recording).as_millis() as f64;
        run_ms as f64 / (run_ms as f64 + cooldown)
    };

    // Typical Vision run times, over the range where the multiplier (not a
    // clamp) governs. That range differs per band because the MAX differs:
    // recording binds at 2.5s (10s / 4.0), catch-up at 1.33s (2s / 1.5).
    for run_ms in [400, 1000, 2000] {
        assert!(
            duty(run_ms, true) <= 0.21,
            "recording duty {} too hot at run_ms={run_ms}",
            duty(run_ms, true)
        );
    }
    for run_ms in [400, 800, 1300] {
        assert!(
            duty(run_ms, false) <= 0.41,
            "catch-up duty {} too hot at run_ms={run_ms}",
            duty(run_ms, false)
        );
    }

    // Pausing capture must never burn *hotter* than recording does — the
    // regression this retune fixed (catch-up was 67% duty against 29%).
    // Catch-up stays the hotter band by design, but within ~2x of recording
    // (the ratio is (1 + 4.0) / (1 + 1.5) = 2.0 exactly), not the 2.3x it was.
    assert!(duty(400, false) > duty(400, true));
    assert!(duty(400, false) <= 2.05 * duty(400, true));

    // ponytail: known ceiling — above ~2.5s (recording) / ~1.3s (catch-up) the
    // MAX clamp binds and the duty cycle rises again, so a pathologically slow
    // frame is not paced to target. Still strictly cooler than before the
    // retune at every run time, so this is a pre-existing property, not a
    // regression. Raise the MAX constants if slow frames ever show up hot.
    assert!(duty(8000, true) > 0.21);
    assert!(duty(8000, false) > 0.41);
}
