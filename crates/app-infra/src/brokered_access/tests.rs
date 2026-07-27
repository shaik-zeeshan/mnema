use super::*;
// Test-only lexical primitives (the non-test code references the other aliases
// at module scope); kept under their historical `recall_*` names.
use crate::lexical::{idf_weight as recall_idf_weight, stem as recall_stem};
use crate::{
    AppInfra, NewAudioSegment, NewFrame, ProcessingJobDraft, ProcessingResultDraft,
    SearchCaptureRefinements, SearchCaptureResponse, SearchDateRangeOrigin,
    SearchDateRangeRefinement,
};

fn run_async_test(test: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(test);
}

fn temp_config_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mnema-brokered-access-{name}-{}-{}",
        std::process::id(),
        now_unix_ms()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn temp_save_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mnema-brokered-access-save-{name}-{}-{}",
        std::process::id(),
        now_unix_ms()
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn write_recording_settings(config_dir: &Path, save_dir: &Path) {
    let settings = RecordingSettingsFile {
        save_directory: save_dir.display().to_string(),
    };
    fs::write(
        config_dir.join(RECORDING_SETTINGS_FILE_NAME),
        serde_json::to_string(&settings).expect("settings should serialize"),
    )
    .expect("settings should write");
}

fn execute_request(
    config_dir: &Path,
    request: BrokeredCaptureRequest,
) -> BrokeredCaptureResponse {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let access = BrokeredCaptureAccess::from_config_dir(config_dir.to_path_buf());
    runtime
        .block_on(access.execute("mnema-cli", request))
        .unwrap()
}

fn test_conclusion(
    subject: &str,
    statement: &str,
    confidence: f64,
    status: capture_types::ConclusionStatus,
) -> capture_types::Conclusion {
    capture_types::Conclusion {
        id: 0,
        subject: subject.to_string(),
        statement: statement.to_string(),
        confidence,
        status,
        pinned: false,
        formed_at_ms: 0,
        last_supported_at_ms: 0,
        updated_at_ms: 0,
        evidence: Vec::new(),
        replaced_statement: None,
        replaced_at_ms: None,
    }
}

fn test_activity(title: &str, summary: &str, started_at_ms: i64) -> capture_types::Activity {
    capture_types::Activity {
        id: 0,
        title: title.to_string(),
        summary: summary.to_string(),
        category: None,
        focus: None,
        started_at_ms,
        ended_at_ms: started_at_ms + 1000,
        created_at_ms: started_at_ms,
        evidence: Vec::new(),
    }
}

#[test]
fn recall_context_drops_sensitive_conclusions() {
    use capture_types::ConclusionStatus::Visible;
    let conclusions = vec![
        test_conclusion("Rust", "Is in a Rust learning phase", 0.9, Visible),
        // Sensitive: must NEVER be returned, even though it matches the query.
        test_conclusion("health", "user has depression", 0.95, Visible),
    ];
    let tokens = recall_query_tokens("tell me about rust and health and depression");
    let recalled = select_relevant_conclusions(&conclusions, &tokens, 10, true);
    assert!(
        recalled.iter().all(|c| !c.statement.contains("depression")),
        "sensitive conclusion leaked: {recalled:?}"
    );
    assert!(recalled.iter().any(|c| c.subject == "Rust"));
}

#[test]
fn recall_context_drops_non_visible_conclusions() {
    use capture_types::ConclusionStatus::{Dismissed, Faded, Visible};
    let conclusions = vec![
        test_conclusion("Rust", "Likes Rust", 0.9, Visible),
        test_conclusion("Rust", "Dismissed Rust opinion", 0.9, Dismissed),
        test_conclusion("Rust", "Faded Rust opinion", 0.9, Faded),
    ];
    let tokens = recall_query_tokens("rust");
    let recalled = select_relevant_conclusions(&conclusions, &tokens, 10, true);
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].statement, "Likes Rust");
}

#[test]
fn recall_context_caps_relevant_conclusions() {
    use capture_types::ConclusionStatus::Visible;
    // 30 relevant, non-sensitive conclusions; the cap must bound the result.
    let conclusions: Vec<_> = (0..30)
        .map(|i| {
            test_conclusion(
                "project alpha",
                &format!("works on project alpha item {i}"),
                0.5,
                Visible,
            )
        })
        .collect();
    let tokens = recall_query_tokens("project alpha");
    let recalled =
        select_relevant_conclusions(&conclusions, &tokens, MAX_RECALL_CONTEXT_LIMIT as usize, true);
    assert_eq!(recalled.len(), MAX_RECALL_CONTEXT_LIMIT as usize);
    assert!(recalled.len() < conclusions.len());
}

#[test]
fn recall_context_empty_query_falls_back_capped_not_whole_dossier() {
    use capture_types::ConclusionStatus::Visible;
    let conclusions: Vec<_> = (0..30)
        .map(|i| test_conclusion("subj", &format!("statement {i}"), i as f64 / 30.0, Visible))
        .collect();
    // Stopwords-only query yields no usable tokens.
    let tokens = recall_query_tokens("what is the and for");
    assert!(tokens.is_empty());
    let recalled = select_relevant_conclusions(&conclusions, &tokens, 5, true);
    assert_eq!(recalled.len(), 5, "fallback must still be capped");
    // Highest confidence first.
    assert!(recalled[0].confidence >= recalled[1].confidence);
}

#[test]
fn recall_context_no_usable_tokens_suppresses_fallback_when_disabled() {
    use capture_types::ConclusionStatus::Visible;
    // The default path falls back to top-by-confidence on a no-token query
    // (above); with the fallback DISABLED (an episodic, time-ranged turn) the
    // SAME no-token query must yield ZERO conclusions instead of a confidence
    // dump of unrelated standing beliefs.
    let conclusions: Vec<_> = (0..10)
        .map(|i| test_conclusion("subj", &format!("statement {i}"), i as f64 / 10.0, Visible))
        .collect();
    let tokens = recall_query_tokens("what is the and for");
    assert!(tokens.is_empty());
    let recalled = select_relevant_conclusions(&conclusions, &tokens, 5, false);
    assert!(
        recalled.is_empty(),
        "fallback disabled must suppress the confidence dump: {recalled:?}"
    );
}

#[test]
fn recall_context_disabling_fallback_does_not_affect_token_query() {
    use capture_types::ConclusionStatus::Visible;
    // With usable tokens the `allow_confidence_fallback` flag has no effect:
    // the normal score>0 path runs regardless of the flag.
    let conclusions = vec![
        test_conclusion("Rust", "Likes Rust", 0.9, Visible),
        test_conclusion("Python", "Likes Python", 0.9, Visible),
    ];
    let tokens = recall_query_tokens("rust");
    let with = select_relevant_conclusions(&conclusions, &tokens, 10, true);
    let without = select_relevant_conclusions(&conclusions, &tokens, 10, false);
    assert_eq!(with.len(), 1);
    assert_eq!(without.len(), 1);
    assert_eq!(with[0].statement, without[0].statement);
    assert_eq!(without[0].subject, "Rust");
}

#[test]
fn recall_context_range_present_caps_conclusions_low() {
    use capture_types::ConclusionStatus::Visible;
    // Many conclusions all match the query, but a present time range caps the
    // recalled set at RANGE_PRESENT_CONCLUSION_LIMIT even though `limit` is
    // higher — proving the episodic de-emphasis.
    let conclusions: Vec<_> = (0..30)
        .map(|i| {
            test_conclusion(
                "project alpha",
                &format!("works on project alpha item {i}"),
                0.5,
                Visible,
            )
        })
        .collect();
    let tokens = recall_query_tokens("project alpha");
    // The handler passes `limit.min(RANGE_PRESENT_CONCLUSION_LIMIT)` and
    // `allow_confidence_fallback = false` when a range is present.
    let limit = (MAX_RECALL_CONTEXT_LIMIT as usize).min(RANGE_PRESENT_CONCLUSION_LIMIT);
    let recalled = select_relevant_conclusions(&conclusions, &tokens, limit, false);
    assert_eq!(recalled.len(), RANGE_PRESENT_CONCLUSION_LIMIT);
}

#[test]
fn recall_context_relevance_filters_and_caps_activities() {
    let activities = vec![
        test_activity("Code review", "Reviewed the parser pull request", 3000),
        test_activity("Lunch break", "Ate a sandwich", 2000),
        test_activity("Parser work", "Wrote a new parser module", 1000),
    ];
    let tokens = recall_query_tokens("parser");
    let recalled = select_relevant_activities(&activities, &tokens, 10);
    assert_eq!(recalled.len(), 2);
    // Both relevant; recency tie-break puts the later one first.
    assert_eq!(recalled[0].title, "Code review");
    assert!(recalled.iter().all(|a| !a.title.contains("Lunch")));
}

// --- #1 whole-word matching: no substring false positives --------------

#[test]
fn recall_word_boundary_matching_rejects_substrings() {
    use capture_types::ConclusionStatus::Visible;
    // Query token "cat" must NOT match "category"/"education" (substring), only
    // the whole word "cat".
    let conclusions = vec![
        test_conclusion("work", "spends time on category triage", 0.9, Visible),
        test_conclusion("pets", "adopted a cat last month", 0.5, Visible),
    ];
    let tokens = recall_query_tokens("cat");
    let recalled = select_relevant_conclusions(&conclusions, &tokens, 10, true);
    assert_eq!(recalled.len(), 1, "only the whole-word match should survive");
    assert_eq!(recalled[0].subject, "pets");
}

#[test]
fn recall_word_boundary_matching_on_activities_rejects_substrings() {
    // "run" must not match "running errands" via substring inside another word,
    // but stemming collapses "running" -> "run", so it SHOULD match as a word.
    let activities = vec![
        test_activity("Prepped a meal", "chopped vegetables for dinner", 2000),
        test_activity("Morning jog", "went running in the park", 1000),
    ];
    let tokens = recall_query_tokens("run");
    let recalled = select_relevant_activities(&activities, &tokens, 10);
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].title, "Morning jog");
}

// --- #2 IDF weighting: rare token outranks common token ----------------

#[test]
fn recall_idf_weight_favors_rare_tokens() {
    // Rarer token (lower df) must weigh more than a common one.
    let rare = recall_idf_weight(100, 1);
    let common = recall_idf_weight(100, 90);
    assert!(rare > common, "rare {rare} should outweigh common {common}");
    // Always positive so any match still counts.
    assert!(recall_idf_weight(100, 100) > 0.0);
}

#[test]
fn recall_idf_ranks_distinctive_match_above_common_match() {
    use capture_types::ConclusionStatus::Visible;
    // "rust" appears in many candidates (common); "kazoo" in one (rare). A
    // single-token query matching the rare word should outrank a single-token
    // query matching the common word, all confidence equal.
    let mut conclusions: Vec<_> = (0..10)
        .map(|i| test_conclusion("rust", &format!("uses rust at work {i}"), 0.5, Visible))
        .collect();
    conclusions.push(test_conclusion(
        "music",
        "plays the kazoo on weekends",
        0.5,
        Visible,
    ));
    // Query both a common and a rare token; the rare-token doc must rank first.
    let tokens = recall_query_tokens("rust kazoo");
    let recalled = select_relevant_conclusions(&conclusions, &tokens, 11, true);
    assert_eq!(
        recalled[0].statement, "plays the kazoo on weekends",
        "rare-token match must rank above common-token matches: {recalled:?}"
    );
}

// --- #3 stemmer: collapses common suffixes, guards short words ---------

#[test]
fn recall_stem_collapses_common_suffixes() {
    // The stem need not be a real word — only consistent. What matters is
    // that morphological variants collapse to the SAME key.
    assert_eq!(recall_stem("running"), "run");
    assert_eq!(recall_stem("tests"), "test");
    assert_eq!(recall_stem("quickly"), "quick");
    assert_eq!(recall_stem("reviewed"), "review");
    // Cross-form keys agree so the matcher bridges the lexical gap.
    assert_eq!(recall_stem("coding"), recall_stem("code"));
    assert_eq!(recall_stem("parsing"), recall_stem("parse"));
    assert_eq!(recall_stem("runs"), recall_stem("running"));
    assert_eq!(recall_stem("tested"), recall_stem("tests"));
}

#[test]
fn recall_stem_guards_against_over_stemming_short_words() {
    // Short words must survive intact rather than being gutted.
    assert_eq!(recall_stem("is"), "is");
    assert_eq!(recall_stem("red"), "red");
    assert_eq!(recall_stem("bus"), "bus");
    assert_eq!(recall_stem("cat"), "cat");
    assert_eq!(recall_stem("ring"), "ring"); // not stemmed to "r"
}

#[test]
fn recall_stemming_bridges_lexical_gap() {
    use capture_types::ConclusionStatus::Visible;
    // Query "running" should reach a conclusion that says "run".
    let conclusions = vec![test_conclusion(
        "fitness",
        "likes to run every morning",
        0.9,
        Visible,
    )];
    let tokens = recall_query_tokens("running");
    let recalled = select_relevant_conclusions(&conclusions, &tokens, 10, true);
    assert_eq!(recalled.len(), 1, "stemming should bridge running~run");
}

// --- #4 sensitive-activity filtering -----------------------------------

#[test]
fn recall_context_drops_sensitive_activities() {
    // An Activity whose title/summary lands in a sensitive category must be
    // dropped before scoring, exactly like sensitive Conclusions.
    let activities = vec![
        test_activity("Therapy session", "attended a therapy appointment", 2000),
        test_activity("Code review", "reviewed the therapy scheduler code", 1000),
    ];
    // Query matches both, but the sensitive one must never be returned.
    let tokens = recall_query_tokens("therapy");
    let recalled = select_relevant_activities(&activities, &tokens, 10);
    assert!(
        recalled.iter().all(|a| a.title != "Therapy session"),
        "sensitive activity leaked: {recalled:?}"
    );
    // The benign code-review activity (its TEXT trips the guardrail via
    // "therapy" too) — confirm guardrail symmetry: anything matching the
    // sensitive term list is dropped, biasing to over-suppression like
    // conclusions do. So NOTHING relevant survives here.
    assert!(recalled.is_empty(), "over-suppression by design: {recalled:?}");
}

#[test]
fn recall_context_keeps_benign_activities_when_sensitive_present() {
    let activities = vec![
        test_activity("Doctor visit", "discussed medication options", 3000),
        test_activity("Parser work", "wrote a new parser module", 2000),
    ];
    let tokens = recall_query_tokens("parser medication");
    let recalled = select_relevant_activities(&activities, &tokens, 10);
    // The medication (sensitive) activity is dropped; the parser one stays.
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].title, "Parser work");
}

// --- fallbacks remain intact (no usable tokens) ------------------------

#[test]
fn recall_context_empty_query_falls_back_most_recent_activities_capped() {
    let activities: Vec<_> = (0..30)
        .map(|i| test_activity(&format!("act {i}"), "summary", 1000 + i as i64))
        .collect();
    // Stopwords-only query yields no usable tokens.
    let tokens = recall_query_tokens("what is the and for");
    assert!(tokens.is_empty());
    let recalled = select_relevant_activities(&activities, &tokens, 5);
    assert_eq!(recalled.len(), 5, "fallback must still be capped");
    // Most-recent first.
    assert!(recalled[0].started_at >= recalled[1].started_at);
}

#[test]
fn recall_context_command_type_and_result_count() {
    let request = BrokeredCaptureRequest::RecallContext(BrokerRecallContextRequest {
        query: "anything".to_string(),
        limit: None,
        from: None,
        to: None,
    });
    assert_eq!(request.command_type(), Some("recall_context"));

    let response = BrokeredCaptureResponse::RecallContext(BrokerRecallContextResponse {
        conclusions: vec![BrokerRecalledConclusion {
            subject: "s".to_string(),
            statement: "t".to_string(),
            confidence: 0.5,
            status: "visible".to_string(),
        }],
        activities: vec![BrokerRecalledActivity {
            title: "a".to_string(),
            summary: "b".to_string(),
            category: None,
            focus: None,
            started_at: "1970-01-01T00:00:00Z".to_string(),
            ended_at: "1970-01-01T00:00:01Z".to_string(),
        }],
    });
    assert_eq!(response.result_count(), 2);
}

/// A seeded in-range Activity that matches the query is recalled; a recent
/// out-of-range Activity that ALSO matches is excluded by the `from`/`to`
/// window. Conclusions are unaffected by the window (they have no wire
/// timestamp). An unparseable bound is IGNORED gracefully — the turn still
/// succeeds with the other (valid) bound applied.
#[test]
fn recall_context_filters_activities_by_time_window_and_ignores_bad_bound() {
    run_async_test(async {
        use crate::user_context::store::{NewActivity, NewActivityEvidence, NewConclusion};

        let config_dir = temp_config_dir("recall-window");
        let save_dir = temp_save_dir("recall-window");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let store = infra.user_context();

        // An OLD activity that matches "parser", inside the window.
        store
            .insert_activity_with_evidence(NewActivity {
                title: "parser internals".to_string(),
                summary: "worked on the parser module".to_string(),
                category: None,
                focus: None,
                started_at_ms: 1_000,
                ended_at_ms: 1_001,
                derivation_run_id: None,
                evidence: vec![NewActivityEvidence {
                    subject_type: "frame".to_string(),
                    subject_id: 1,
                    captured_at_ms: Some(1_000),
                    is_headline: false,
                }],
            })
            .await
            .expect("seed old activity");
        // A RECENT activity that ALSO matches "parser", but outside the window.
        store
            .insert_activity_with_evidence(NewActivity {
                title: "parser rewrite".to_string(),
                summary: "rewrote the parser".to_string(),
                category: None,
                focus: None,
                started_at_ms: 50_000,
                ended_at_ms: 50_001,
                derivation_run_id: None,
                evidence: vec![NewActivityEvidence {
                    subject_type: "frame".to_string(),
                    subject_id: 2,
                    captured_at_ms: Some(50_000),
                    is_headline: false,
                }],
            })
            .await
            .expect("seed recent activity");
        // A visible Conclusion that mentions the query subject — must survive
        // regardless of the time window (Conclusions are never time-scoped).
        store
            .upsert_conclusion(NewConclusion {
                subject: "parser".to_string(),
                statement: "The user maintains a parser".to_string(),
                confidence: 0.9,
                formed_at_ms: 1_000,
                last_supported_at_ms: 1_000,
            })
            .await
            .expect("seed conclusion");

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        // Window [500ms, 10_000ms): one valid `from`, and a deliberately
        // BAD `to` that must be ignored gracefully (the turn still runs with
        // only `from` applied, so the recent out-of-range match survives —
        // see the assertion below for the both-valid-bounds case).
        let bad_to = broker_recall_context(
            &infra,
            &[grant.clone()],
            BrokerRecallContextRequest {
                query: "parser".to_string(),
                limit: None,
                from: Some("1970-01-01T00:00:00.500Z".to_string()),
                to: Some("not-a-timestamp".to_string()),
            },
        )
        .await
        .expect("recall should run")
        .expect("recall should be authorized");
        // Bad `to` ignored → only `from` applied → BOTH parser activities
        // survive (turn did not error).
        assert_eq!(bad_to.activities.len(), 2, "{:?}", bad_to.activities);

        // Both bounds valid: window [500ms, 10_000ms) catches only the old
        // parser activity; the recent one is excluded.
        let windowed = broker_recall_context(
            &infra,
            &[grant],
            BrokerRecallContextRequest {
                query: "parser".to_string(),
                limit: None,
                from: Some("1970-01-01T00:00:00.500Z".to_string()),
                to: Some("1970-01-01T00:00:10Z".to_string()),
            },
        )
        .await
        .expect("recall should run")
        .expect("recall should be authorized");

        assert_eq!(windowed.activities.len(), 1, "{:?}", windowed.activities);
        assert_eq!(windowed.activities[0].title, "parser internals");
        // Conclusion is unaffected by the activity time window.
        assert_eq!(windowed.conclusions.len(), 1);
        assert_eq!(windowed.conclusions[0].subject, "parser");
    });
}

/// End-to-end regression (#4): an Activity is persisted *unfiltered*, so the
/// ONLY guardrail on the Activity egress path is the broker re-filter in
/// `select_relevant_activities`. Drive the full `broker_recall_context` over a
/// real store and assert a sensitive Activity never appears in
/// `BrokerRecallContextResponse.activities`. This is the test the load-bearing
/// comment points at — if someone deletes the "redundant"-looking filter line,
/// THIS goes red even though derivation-time tests stay green.
#[test]
fn sensitive_activity_never_egresses_via_recall_context() {
    run_async_test(async {
        use crate::user_context::store::{NewActivity, NewActivityEvidence};

        let config_dir = temp_config_dir("recall-sensitive-activity");
        let save_dir = temp_save_dir("recall-sensitive-activity");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let store = infra.user_context();

        // A SENSITIVE activity persisted unfiltered (derivation does NOT drop
        // Activities), matching the query on a benign token ("appointment").
        store
            .insert_activity_with_evidence(NewActivity {
                title: "Therapy appointment".to_string(),
                summary: "attended a therapy appointment".to_string(),
                category: None,
                focus: None,
                started_at_ms: 2_000,
                ended_at_ms: 2_001,
                derivation_run_id: None,
                evidence: vec![NewActivityEvidence {
                    subject_type: "frame".to_string(),
                    subject_id: 1,
                    captured_at_ms: Some(2_000),
                    is_headline: false,
                }],
            })
            .await
            .expect("seed sensitive activity");
        // A benign activity matching the same query token, to prove recall is
        // working (not just empty) while the sensitive one is excluded.
        store
            .insert_activity_with_evidence(NewActivity {
                title: "Dentist appointment".to_string(),
                summary: "booked a dentist appointment".to_string(),
                category: None,
                focus: None,
                started_at_ms: 1_000,
                ended_at_ms: 1_001,
                derivation_run_id: None,
                evidence: vec![NewActivityEvidence {
                    subject_type: "frame".to_string(),
                    subject_id: 2,
                    captured_at_ms: Some(1_000),
                    is_headline: false,
                }],
            })
            .await
            .expect("seed benign activity");

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_recall_context(
            &infra,
            &[grant],
            BrokerRecallContextRequest {
                query: "appointment".to_string(),
                limit: None,
                from: None,
                to: None,
            },
        )
        .await
        .expect("recall should run")
        .expect("recall should be authorized");

        // The sensitive activity must NOT appear in the response, in neither
        // title nor summary (no sensitive text crosses the boundary).
        assert!(
            response.activities.iter().all(|a| {
                !crate::user_context::guardrail::is_sensitive(&a.title, &a.summary)
            }),
            "sensitive activity egressed via recall_context: {:?}",
            response.activities
        );
        assert!(
            response
                .activities
                .iter()
                .all(|a| a.title != "Therapy appointment"),
            "therapy activity leaked: {:?}",
            response.activities
        );
        // The benign appointment still comes back — recall is genuinely working.
        assert!(
            response
                .activities
                .iter()
                .any(|a| a.title == "Dentist appointment"),
            "benign activity should still be recalled: {:?}",
            response.activities
        );
    });
}

/// A range-present query with NO usable tokens (all stopwords) returns ZERO
/// conclusions — the no-token confidence fallback is suppressed for episodic
/// turns — while still returning the date-filtered activities. This proves
/// Conclusions are de-emphasized without harming the activity timeline that
/// actually answers an episodic question.
#[test]
fn recall_context_range_present_no_tokens_drops_conclusions_keeps_activities() {
    run_async_test(async {
        use crate::user_context::store::{NewActivity, NewActivityEvidence, NewConclusion};

        let config_dir = temp_config_dir("recall-range-no-tokens");
        let save_dir = temp_save_dir("recall-range-no-tokens");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let store = infra.user_context();

        // An in-window activity (any text; the query has no usable tokens, so
        // the activity path degrades to the most-recent in-window set).
        store
            .insert_activity_with_evidence(NewActivity {
                title: "morning standup".to_string(),
                summary: "discussed the sprint plan".to_string(),
                category: None,
                focus: None,
                started_at_ms: 1_000,
                ended_at_ms: 1_001,
                derivation_run_id: None,
                evidence: vec![NewActivityEvidence {
                    subject_type: "frame".to_string(),
                    subject_id: 1,
                    captured_at_ms: Some(1_000),
                    is_headline: false,
                }],
            })
            .await
            .expect("seed in-window activity");
        // A high-confidence standing belief that the OLD (rangeless) path would
        // have dumped via the no-token confidence fallback.
        store
            .upsert_conclusion(NewConclusion {
                subject: "habits".to_string(),
                statement: "The user prefers dark mode".to_string(),
                confidence: 0.99,
                formed_at_ms: 1_000,
                last_supported_at_ms: 1_000,
            })
            .await
            .expect("seed conclusion");

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        // Stopwords-only query → no usable tokens. A present range (valid
        // bounds) suppresses the confidence fallback.
        let ranged = broker_recall_context(
            &infra,
            &[grant],
            BrokerRecallContextRequest {
                query: "what did i do".to_string(),
                limit: None,
                from: Some("1970-01-01T00:00:00.500Z".to_string()),
                to: Some("1970-01-01T00:00:10Z".to_string()),
            },
        )
        .await
        .expect("recall should run")
        .expect("recall should be authorized");

        assert!(
            ranged.conclusions.is_empty(),
            "range-present no-token query must drop conclusions: {:?}",
            ranged.conclusions
        );
        // Activities intact: the in-window activity still comes back.
        assert_eq!(ranged.activities.len(), 1, "{:?}", ranged.activities);
        assert_eq!(ranged.activities[0].title, "morning standup");
    });
}

#[test]
fn capture_request_without_active_grants_returns_authorization_error_without_audit() {
    let config_dir = temp_config_dir("no-grants");

    let response = execute_request(
        &config_dir,
        BrokeredCaptureRequest::Search(BrokerSearchRequest {
            query: "meeting".to_string(),
            from: None,
            to: None,
            limit: Some(5),
            app: None,
            window_title: None,
            url: None,
            url_regex: None,
            cursor: None,
        }),
    );

    assert_eq!(
        response,
        BrokeredCaptureResponse::Error(BrokerErrorResponse::authorization_required())
    );
    assert!(load_audit_events(&config_dir).unwrap().events.is_empty());
}

#[test]
fn invalid_open_request_is_shaped_and_audited_by_brokered_capture_access() {
    let config_dir = temp_config_dir("invalid-open");
    create_grant_from_request(
        &config_dir,
        BrokerGrantCreateRequest {
            label: Some("Local agent".to_string()),
            duration_hours: Some(1),
            all_retained_history: Some(false),
        },
    )
    .unwrap();

    let response = execute_request(
        &config_dir,
        BrokeredCaptureRequest::OpenInMnema {
            opaque_id: "not-valid".to_string(),
        },
    );

    assert_eq!(
        response,
        BrokeredCaptureResponse::Error(BrokerErrorResponse {
            error: BrokerAuthStatusKind::AuthorizationRequired,
            message: "invalid opaque result id".to_string(),
        })
    );
    let audit = load_audit_events(&config_dir).unwrap();
    assert_eq!(audit.events.len(), 1);
    assert_eq!(audit.events[0].tool_identity, "mnema-cli");
    assert_eq!(audit.events[0].command_type, "open_in_mnema");
    assert_eq!(audit.events[0].result_count, 0);
    assert_eq!(audit.events[0].scope_class, "time_scoped");
}

#[test]
fn grant_create_request_applies_default_label_and_duration_cap() {
    let config_dir = temp_config_dir("create-grant");

    let grant = create_grant_from_request(
        &config_dir,
        BrokerGrantCreateRequest {
            label: None,
            duration_hours: Some(24 * 31),
            all_retained_history: Some(true),
        },
    )
    .unwrap();

    assert_eq!(grant.label, "Local agent");
    assert_eq!(grant.scope, BrokerGrantScope::AllRetainedHistory);
    assert_eq!(
        grant.expires_at_unix_ms - grant.created_at_unix_ms,
        24 * 30 * 60 * 60 * 1000
    );
}

#[test]
fn app_identifier_config_dir_uses_supplied_identifier() {
    if std::env::var_os("MNEMA_APP_CONFIG_DIR").is_some() {
        return;
    }
    let path = default_app_config_dir_for_identifier("com.example.mnema-test")
        .expect("config dir should resolve");

    assert!(path.ends_with("com.example.mnema-test"));
}

#[test]
fn empty_opaque_id_is_invalid_instead_of_panicking() {
    assert_eq!(decode_opaque_id(""), None);
    assert_eq!(opaque_capture_reference(""), None);
}

#[test]
fn signed_opaque_capture_reference_requires_broker_signature() {
    let config_dir = temp_config_dir("signed-opaque-reference");
    let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
    let opaque_id = encode_signed_opaque_id("frame", 17, Some("grant-1"), &secret);

    assert_eq!(
        signed_opaque_capture_reference(&config_dir, "f11").expect("unsigned should parse"),
        None
    );
    assert_eq!(
        signed_opaque_capture_reference(&config_dir, &opaque_id).expect("signed should parse"),
        Some(BrokerOpaqueCaptureReference {
            opaque_id,
            frame_id: Some(17),
            audio_segment_id: None,
            grant_id: Some("grant-1".to_string()),
            kind: "frame".to_string(),
        })
    );
}

#[test]
fn broker_search_page_mixes_kinds_by_rank_before_applying_limit() {
    let secret = b"test broker opaque secret with enough bytes";
    let response = two_frame_one_audio_response();
    let mapped = map_search_response(response, 2, None, Some("grant-1"), secret);

    assert_eq!(
        mapped
            .results
            .iter()
            .map(|result| result.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["frame", "audio_microphone"]
    );
    assert!(mapped.results[0].opaque_id.contains('.'));
    assert_ne!(mapped.results[0].opaque_id, "fb");
    assert_eq!(
        mapped.results[0].context,
        Some(BrokerSearchResultContext {
            app_bundle_id: Some("com.example.Linear".to_string()),
            app_name: Some("Linear".to_string()),
            window_title: Some("Roadmap".to_string()),
            url: None,
        })
    );
    assert_eq!(mapped.results[1].context, None);
    // Ranks are frame -5, audio -3, frame -1: the page takes the best frame
    // and the audio hit, leaving the weakest frame behind. The next page must
    // resume at frame 1 / audio 1 — what was CONSUMED — not at `offset +
    // limit` (2/2), which would silently skip that leftover frame.
    assert_eq!(mapped.next_cursor.as_deref(), Some("v1:1:1:1"));
}

/// Two frame groups + one audio group, the shape `map_search_response` has
/// to merge. Ranks are placeholders — every test that cares sets them.
fn two_frame_one_audio_response() -> SearchCaptureResponse {
    let frame = |id: i64| crate::Frame {
        id,
        session_id: "screen-session".to_string(),
        file_path: format!("/tmp/frame-{id}.jpg"),
        captured_at: "2026-05-17T10:00:00Z".to_string(),
        width: None,
        height: None,
        equivalence: crate::FrameEquivalence {
            hint: None,
            proof: None,
            version: None,
            status: None,
            error: None,
        },
        metadata_snapshot: None,
        created_at: "2026-05-17T10:00:00Z".to_string(),
        updated_at: "2026-05-17T10:00:00Z".to_string(),
    };
    let audio_segment = crate::AudioSegment {
        id: 22,
        source_kind: AudioSegmentSourceKind::Microphone,
        source_session_id: "mic-session".to_string(),
        segment_index: 1,
        file_path: "/tmp/audio.m4a".to_string(),
        started_at: "2026-05-17T10:00:00Z".to_string(),
        ended_at: "2026-05-17T10:00:20Z".to_string(),
        capture_segment_id: None,
        created_at: "2026-05-17T10:00:00Z".to_string(),
        updated_at: "2026-05-17T10:00:00Z".to_string(),
    };
    SearchCaptureResponse {
        normalized_query: "target".to_string(),
        snapshot_document_id: 1,
        frames: vec![
            crate::FrameSearchResult {
                rank: -5.0,
                group_key: "frame:11".to_string(),
                representative_frame: frame(11),
                group_start_at: "2026-05-17T10:00:00Z".to_string(),
                group_end_at: "2026-05-17T10:00:00Z".to_string(),
                match_count: 1,
                snippet: "frame target".to_string(),
                app_bundle_id: Some("com.example.Linear".to_string()),
                app_name: Some("Linear".to_string()),
                window_title: Some("Roadmap".to_string()),
                browser_url: None,
                thumbnail_frame_id: 11,
                text_source_kind: "direct".to_string(),
                secret_redaction_count: 0,
                has_secret_redactions: false,
                found_by_meaning: false,
            },
            crate::FrameSearchResult {
                rank: -1.0,
                group_key: "frame:12".to_string(),
                representative_frame: frame(12),
                group_start_at: "2026-05-17T10:01:00Z".to_string(),
                group_end_at: "2026-05-17T10:01:00Z".to_string(),
                match_count: 1,
                snippet: "second frame target".to_string(),
                app_bundle_id: None,
                app_name: None,
                window_title: None,
                browser_url: None,
                thumbnail_frame_id: 12,
                text_source_kind: "direct".to_string(),
                secret_redaction_count: 0,
                has_secret_redactions: false,
                found_by_meaning: false,
            },
        ],
        audio: vec![crate::AudioSearchResult {
            rank: -3.0,
            group_key: "audio:22:0-1000".to_string(),
            audio_segment,
            source_kind: AudioSegmentSourceKind::Microphone,
            span_start_ms: 0,
            span_end_ms: 1_000,
            absolute_start_at: "2026-05-17T10:00:00Z".to_string(),
            absolute_end_at: "2026-05-17T10:00:01Z".to_string(),
            match_count: 1,
            snippet: "audio target".to_string(),
            aligned_frame: None,
            secret_redaction_count: 0,
            has_secret_redactions: false,
            found_by_meaning: false,
        }],
        has_more_frames: false,
        has_more_audio: false,
        applied_refinements: SearchCaptureRefinements {
            date_range: Some(SearchDateRangeRefinement {
                start_at: "2026-05-17T00:00:00Z".to_string(),
                end_at: "2026-05-18T00:00:00Z".to_string(),
                origin: Some(SearchDateRangeOrigin::VisibleTimeline),
            }),
            apps: Vec::new(),
            window_title: None,
            url: None,
            url_regex: None,
            audio_sources: Vec::new(),
            screen_source: false,
        },
        residual_query: "target".to_string(),
        parse_errors: Vec::new()
    }
}

#[test]
fn search_page_is_ranked_across_both_anchor_kinds_not_alternated() {
    let secret = &[7u8; 32];
    // Both frames outrank the audio result (-9 / -8 beat -3), so a ranked
    // page of 2 is all frames. Alternating would have surfaced the weaker
    // audio hit ahead of the second-best frame.
    let mut response = two_frame_one_audio_response();
    response.frames[0].rank = -9.0;
    response.frames[1].rank = -8.0;
    response.audio[0].rank = -3.0;
    let mapped = map_search_response(response, 2, None, Some("grant-1"), secret);
    assert_eq!(
        mapped
            .results
            .iter()
            .map(|result| result.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["frame", "frame"]
    );
    // Nothing audio was consumed, so only the frame offset advances.
    assert_eq!(mapped.next_cursor.as_deref(), Some("v1:1:2:0"));

    // Same rows, audio now the strongest hit: it leads the page.
    let mut response = two_frame_one_audio_response();
    response.frames[0].rank = -2.0;
    response.frames[1].rank = -1.0;
    response.audio[0].rank = -9.0;
    let mapped = map_search_response(response, 2, None, Some("grant-1"), secret);
    assert_eq!(
        mapped
            .results
            .iter()
            .map(|result| result.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["audio_microphone", "frame"]
    );
    assert_eq!(mapped.next_cursor.as_deref(), Some("v1:1:1:1"));
}

/// A `limit: 0` page can never emit a row, so a cursor on it promises progress
/// that re-sending it can never make: the agent contract is "page until
/// `nextCursor` is absent" (`.agents/skills/mnema-data/SKILL.md`), and the
/// identical request+cursor returns the identical empty page forever.
#[test]
fn zero_limit_search_page_hands_back_no_cursor_to_loop_on() {
    let mapped = map_search_response(
        two_frame_one_audio_response(),
        0,
        None,
        Some("grant-1"),
        &[7u8; 32],
    );
    assert!(mapped.results.is_empty(), "limit 0 emits no rows");
    assert_eq!(
        mapped.next_cursor, None,
        "a page that can never emit a row must end the walk, not loop"
    );
}

#[test]
fn search_cursor_resumes_from_consumed_anchors_and_stops_at_the_end() {
    let secret = &[7u8; 32];
    let exhausted = map_search_response(
        frame_search_response_with_browser_url(11, None),
        5,
        Some(BrokerSearchCursor {
            snapshot_document_id: 9,
            frame_offset: 4,
            audio_offset: 2,
        }),
        Some("grant-1"),
        secret,
    );
    assert_eq!(
        exhausted.next_cursor, None,
        "a page that emitted everything available ends the walk"
    );

    // Same page, but the search layer says more frames remain: the cursor
    // advances by what THIS page consumed and stays pinned to the snapshot
    // the walk started on, not the fresher one in the response.
    let mut response = frame_search_response_with_browser_url(11, None);
    response.snapshot_document_id = 77;
    response.has_more_frames = true;
    let paged = map_search_response(
        response,
        5,
        Some(BrokerSearchCursor {
            snapshot_document_id: 9,
            frame_offset: 4,
            audio_offset: 2,
        }),
        Some("grant-1"),
        secret,
    );
    assert_eq!(paged.next_cursor.as_deref(), Some("v1:9:5:2"));
}

#[test]
fn search_cursor_round_trips_and_rejects_garbage() {
    let cursor = BrokerSearchCursor {
        snapshot_document_id: 42,
        frame_offset: 7,
        audio_offset: 0,
    };
    assert_eq!(cursor.encode(), "v1:42:7:0");
    assert_eq!(BrokerSearchCursor::decode("v1:42:7:0").unwrap(), cursor);
    for bad in [
        "",
        "42:7:0",
        "v2:42:7:0",
        "v1:42:7",
        "v1:42:7:0:1",
        "v1:-1:7:0",
        "v1:a:7:0",
    ] {
        assert!(
            BrokerSearchCursor::decode(bad).is_err(),
            "{bad} should be rejected"
        );
    }
}

/// Mint a `Frame` for a search-result fixture. The `browser_url` is carried on
/// the `FrameSearchResult` (read-time from the representative snapshot), not on
/// this `Frame`; `map_search_response` only reads the result-level field.
fn search_result_frame(id: i64) -> crate::Frame {
    crate::Frame {
        id,
        session_id: "screen-session".to_string(),
        file_path: format!("/tmp/frame-{id}.jpg"),
        captured_at: "2026-05-17T10:00:00Z".to_string(),
        width: None,
        height: None,
        equivalence: crate::FrameEquivalence {
            hint: None,
            proof: None,
            version: None,
            status: None,
            error: None,
        },
        metadata_snapshot: None,
        created_at: "2026-05-17T10:00:00Z".to_string(),
        updated_at: "2026-05-17T10:00:00Z".to_string(),
    }
}

/// Build a single-frame `SearchCaptureResponse` whose representative frame
/// carries `browser_url`. This mirrors what `search.rs` populates read-time
/// from the representative frame's metadata snapshot.
fn frame_search_response_with_browser_url(
    id: i64,
    browser_url: Option<&str>,
) -> SearchCaptureResponse {
    SearchCaptureResponse {
        normalized_query: "target".to_string(),
        snapshot_document_id: 1,
        frames: vec![crate::FrameSearchResult {
            rank: -1.0,
            group_key: format!("frame:{id}"),
            representative_frame: search_result_frame(id),
            group_start_at: "2026-05-17T10:00:00Z".to_string(),
            group_end_at: "2026-05-17T10:00:00Z".to_string(),
            match_count: 1,
            snippet: "frame target".to_string(),
            app_bundle_id: Some("com.google.Chrome".to_string()),
            app_name: Some("Google Chrome".to_string()),
            window_title: Some("Tab".to_string()),
            browser_url: browser_url.map(str::to_string),
            thumbnail_frame_id: id,
            text_source_kind: "direct".to_string(),
            secret_redaction_count: 0,
            has_secret_redactions: false,
            found_by_meaning: false,
        }],
        audio: Vec::new(),
        has_more_frames: false,
        has_more_audio: false,
        applied_refinements: SearchCaptureRefinements::default(),
        residual_query: "target".to_string(),
        parse_errors: Vec::new(),
    }
}

#[test]
fn broker_search_frame_url_is_guarded_preserving_commit_sha() {
    // Historical-coverage proof: a frame whose snapshot carries a browser_url
    // is mapped at broker-return time regardless of when it was captured —
    // there is no index column or backfill, so any frame with a snapshot
    // browser_url is covered for free. A commit SHA must survive the guard.
    let secret = b"test broker opaque secret with enough bytes";
    let response = frame_search_response_with_browser_url(
        11,
        Some("https://github.com/owner/repo/commit/9fceb02d8f1c3b4a5e6d7c8b9a0f1e2d3c4b5a6f"),
    );

    let mapped = map_search_response(response, 5, None, Some("grant-1"), secret);

    let context = mapped.results[0]
        .context
        .as_ref()
        .expect("frame result should carry app/window context");
    assert_eq!(
        context.url.as_deref(),
        Some("github.com/owner/repo/commit/9fceb02d8f1c3b4a5e6d7c8b9a0f1e2d3c4b5a6f"),
    );
}

#[test]
fn broker_search_frame_url_redacts_armed_token_segment() {
    let secret = b"test broker opaque secret with enough bytes";
    let response = frame_search_response_with_browser_url(
        11,
        Some("https://site.com/reset-password/AbC9xK2mP4qR7sT0"),
    );

    let mapped = map_search_response(response, 5, None, Some("grant-1"), secret);

    let url = mapped.results[0]
        .context
        .as_ref()
        .and_then(|context| context.url.as_deref())
        .expect("guarded url should be present");
    assert!(
        url.contains("reset-password"),
        "credential keyword stays visible: {url}"
    );
    assert!(
        !url.contains("AbC9xK2mP4qR7sT0"),
        "armed token must be redacted: {url}"
    );
}

#[test]
fn broker_search_frame_without_browser_url_has_context_but_no_url() {
    let secret = b"test broker opaque secret with enough bytes";
    let response = frame_search_response_with_browser_url(11, None);

    let mapped = map_search_response(response, 5, None, Some("grant-1"), secret);

    let context = mapped.results[0]
        .context
        .as_ref()
        .expect("app/window context should still be present");
    assert_eq!(context.app_name.as_deref(), Some("Google Chrome"));
    assert_eq!(context.url, None);
}

#[test]
fn broker_search_audio_result_has_no_context_or_url() {
    let secret = b"test broker opaque secret with enough bytes";
    let audio_segment = crate::AudioSegment {
        id: 22,
        source_kind: AudioSegmentSourceKind::Microphone,
        source_session_id: "mic-session".to_string(),
        segment_index: 1,
        file_path: "/tmp/audio.m4a".to_string(),
        started_at: "2026-05-17T10:00:00Z".to_string(),
        ended_at: "2026-05-17T10:00:20Z".to_string(),
        capture_segment_id: None,
        created_at: "2026-05-17T10:00:00Z".to_string(),
        updated_at: "2026-05-17T10:00:00Z".to_string(),
    };
    let response = SearchCaptureResponse {
        normalized_query: "target".to_string(),
        snapshot_document_id: 1,
        frames: Vec::new(),
        audio: vec![crate::AudioSearchResult {
            rank: -3.0,
            group_key: "audio:22:0-1000".to_string(),
            audio_segment,
            source_kind: AudioSegmentSourceKind::Microphone,
            span_start_ms: 0,
            span_end_ms: 1_000,
            absolute_start_at: "2026-05-17T10:00:00Z".to_string(),
            absolute_end_at: "2026-05-17T10:00:01Z".to_string(),
            match_count: 1,
            snippet: "audio target".to_string(),
            aligned_frame: None,
            secret_redaction_count: 0,
            has_secret_redactions: false,
            found_by_meaning: false,
        }],
        has_more_frames: false,
        has_more_audio: false,
        applied_refinements: SearchCaptureRefinements::default(),
        residual_query: "target".to_string(),
        parse_errors: Vec::new(),
    };

    let mapped = map_search_response(response, 5, None, Some("grant-1"), secret);

    assert_eq!(mapped.results[0].kind, "audio_microphone");
    assert_eq!(mapped.results[0].context, None);
}


/// The cursor is UNSIGNED, so its `frame_offset` is attacker-chosen. It feeds
/// `needed_groups = offset + limit + 1` in the search layer's frame drain loop,
/// which then keeps fetching 5_000 hits at a time — re-grouping the whole
/// accumulated list (quadratic) on every iteration — until the ENTIRE match set
/// for the query is materialized in memory. A page the broker itself issued can
/// only ever advance the offset by what that page consumed, so an offset that
/// large is not a cursor the boundary can have minted: it must be refused
/// rather than executed.
#[test]
fn broker_search_refuses_a_forged_cursor_offset_it_could_never_have_issued() {
    run_async_test(async {
        let config_dir = temp_config_dir("search-forged-offset");
        let save_dir = temp_save_dir("search-forged-offset");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        for index in 0..3 {
            seed_timeline_frame_with_browser_url(
                &infra,
                &save_dir,
                &format!("forged-offset-{index}.jpg"),
                &format!("2026-05-17T10:0{index}:00Z"),
                None,
            )
            .await;
        }

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let page = |cursor: Option<String>| {
            let infra = &infra;
            let grant = grant.clone();
            let config_dir = config_dir.clone();
            async move {
                broker_search(
                    &config_dir,
                    infra,
                    &[grant],
                    BrokerSearchRequest {
                        query: "timeline".to_string(),
                        from: None,
                        to: None,
                        limit: Some(1),
                        app: None,
                        window_title: None,
                        url: None,
                        url_regex: None,
                        cursor,
                    },
                )
                .await
            }
        };

        let first = page(None)
            .await
            .expect("search should run")
            .expect("search should be authorized");
        let issued = BrokerSearchCursor::decode(
            first
                .next_cursor
                .as_deref()
                .expect("a first page of 1 leaves more to walk"),
        )
        .expect("the broker's own cursor should decode");
        assert!(
            issued.frame_offset <= 1,
            "the broker only ever advances the offset by what a page consumed, got {issued:?}"
        );

        // Same walk, same query — only the offset is forged.
        let forged = BrokerSearchCursor {
            snapshot_document_id: issued.snapshot_document_id,
            frame_offset: u32::MAX,
            audio_offset: 0,
        }
        .encode();
        let error = page(Some(forged))
            .await
            .expect_err("a forged paging offset must be refused, not executed");
        assert!(
            matches!(error, AppInfraError::InvalidSearchRequest(_)),
            "expected an invalid-request refusal, got {error:?}"
        );

        // The honest cursor still pages, so the guard is not a blanket refusal.
        let second = page(first.next_cursor.clone())
            .await
            .expect("search should run")
            .expect("search should be authorized");
        assert_eq!(second.results.len(), 1, "the honest walk must keep working");
    });
}

#[test]
fn broker_timeline_filters_screen_intervals_by_app_and_window_title() {
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-context");
        let save_dir = temp_save_dir("timeline-context");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        for (file_name, captured_at, window_title) in [
            (
                "timeline-roadmap.jpg",
                "2026-05-17T10:00:00Z",
                "Roadmap Grooming",
            ),
            ("timeline-planning.jpg", "2026-05-17T10:01:00Z", "Planning"),
        ] {
            let frame = infra
                .insert_frame(
                    &NewFrame::new(
                        "screen-session",
                        save_dir.join(file_name).display().to_string(),
                        captured_at,
                    )
                    .with_metadata_snapshot(
                        capture_metadata::FrameMetadataSnapshot {
                            app_bundle_id: Some("com.example.Linear".to_string()),
                            app_name: Some("Linear".to_string()),
                            window_title: Some(window_title.to_string()),
                            window_id: None,
                            browser_url: None,
                            display_id: Some(1),
                            metadata_redaction_reason: None,
                            metadata_redaction_source_id: None,
                        },
                    ),
                )
                .await
                .expect("frame should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
                .await
                .expect("OCR job should enqueue");
            let running = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("OCR job should claim")
                .expect("OCR job should exist");
            infra
                .complete_processing_job(
                    running.id,
                    &ProcessingResultDraft::new().with_result_text("timeline body"),
                )
                .await
                .expect("OCR job should complete");
        }

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_timeline(
            &config_dir,
            &infra,
            &[grant],
            BrokerTimelineRequest {
                from: "2026-05-17T00:00:00Z".to_string(),
                to: "2026-05-18T00:00:00Z".to_string(),
                limit: Some(5),
                app: Some("Linear".to_string()),
                window_title: Some("roadmap".to_string()),
                url: None,
                url_regex: None,
            },
        )
        .await
        .expect("timeline should run")
        .expect("timeline should be authorized");

        assert_eq!(response.intervals.len(), 1);
        assert_eq!(response.intervals[0].kind, "frame");
        assert_eq!(
            response.intervals[0]
                .context
                .as_ref()
                .and_then(|context| context.app_name.as_deref()),
            Some("Linear")
        );
        assert_eq!(
            response.intervals[0]
                .context
                .as_ref()
                .and_then(|context| context.window_title.as_deref()),
            Some("Roadmap Grooming")
        );
    });
}

/// Seed one OCR'd frame (its metadata snapshot optionally carrying
/// `browser_url`) so it lands a `search_documents` frame row the timeline can
/// group. Returns the inserted frame id. Mirrors the OCR enqueue→claim→complete
/// dance the other timeline tests use to project a frame into `search_documents`.
async fn seed_timeline_frame_with_browser_url(
    infra: &AppInfra,
    save_dir: &std::path::Path,
    file_name: &str,
    captured_at: &str,
    browser_url: Option<&str>,
) -> i64 {
    let frame = infra
        .insert_frame(
            &NewFrame::new(
                "screen-session",
                save_dir.join(file_name).display().to_string(),
                captured_at,
            )
            .with_metadata_snapshot(capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.google.Chrome".to_string()),
                app_name: Some("Google Chrome".to_string()),
                window_title: Some("Pull Request".to_string()),
                window_id: None,
                browser_url: browser_url.map(str::to_string),
                display_id: Some(1),
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            }),
        )
        .await
        .expect("frame should insert");
    let job = infra
        .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
        .await
        .expect("OCR job should enqueue");
    let running = infra
        .claim_queued_processing_job(job.id)
        .await
        .expect("OCR job should claim")
        .expect("OCR job should exist");
    infra
        .complete_processing_job(
            running.id,
            &ProcessingResultDraft::new().with_result_text("timeline body"),
        )
        .await
        .expect("OCR job should complete");
    frame.id
}

// REGRESSION (deep-review finding): one corrupt/legacy `snapshot_json` row
// must NOT make the batched `get_frame_metadata_snapshots` fail the WHOLE
// load. The broker timeline batch-loads every interval's representative-frame
// snapshot through this loader; with `serde_json::from_str(&json)?` a single
// present-but-malformed snapshot `?`-propagated and errored the entire
// interactive timeline page (up to MAX_SEARCH_LIMIT intervals), dropping
// every other interval's URL too. The loader's own doc says only frames with
// NO snapshot are absent from the map — a corrupt snapshot must degrade the
// SAME way (frame absent, URL None), not poison the page.
#[test]
fn get_frame_metadata_snapshots_skips_corrupt_row_instead_of_failing_page() {
    run_async_test(async {
        let save_dir = temp_save_dir("snapshot-corrupt-skip");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        let snapshot = |title: &str, url: &str| {
            capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.google.Chrome".to_string()),
                app_name: Some("Google Chrome".to_string()),
                window_title: Some(title.to_string()),
                window_id: None,
                browser_url: Some(url.to_string()),
                display_id: Some(1),
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            }
        };
        let good = infra
            .insert_frame(
                &NewFrame::new(
                    "screen-session",
                    save_dir.join("good.jpg").display().to_string(),
                    "2026-05-17T10:00:00Z",
                )
                .with_metadata_snapshot(snapshot("Good", "https://example.com/good")),
            )
            .await
            .expect("good frame should insert");
        let corrupt = infra
            .insert_frame(
                &NewFrame::new(
                    "screen-session",
                    save_dir.join("corrupt.jpg").display().to_string(),
                    "2026-05-17T10:01:00Z",
                )
                .with_metadata_snapshot(snapshot("Corrupt", "https://example.com/corrupt")),
            )
            .await
            .expect("corrupt frame should insert");

        // Corrupt the second frame's snapshot_json to non-JSON. It stays
        // non-empty so it still passes the table's
        // `LENGTH(TRIM(snapshot_json)) > 0` CHECK, but cannot deserialize.
        sqlx::query(
            "UPDATE frame_metadata_snapshots \
             SET snapshot_json = 'not valid json at all' \
             WHERE id = (SELECT metadata_snapshot_id FROM frames WHERE id = ?1)",
        )
        .bind(corrupt.id)
        .execute(infra.pool())
        .await
        .expect("snapshot json should corrupt");

        let snapshots = infra
            .get_frame_metadata_snapshots(&[good.id, corrupt.id])
            .await
            .expect("one corrupt snapshot row must not fail the whole batch load");

        assert_eq!(
            snapshots
                .get(&good.id)
                .and_then(|snapshot| snapshot.browser_url.as_deref()),
            Some("https://example.com/good"),
            "the good frame's snapshot must still resolve"
        );
        assert!(
            !snapshots.contains_key(&corrupt.id),
            "the corrupt frame must degrade to absent, not poison the page"
        );
    });
}

#[test]
fn broker_timeline_interval_carries_guarded_url_of_representative_frame() {
    // Page-granular accuracy + read-time URL guard: the interval's url is the
    // representative frame's captured `browser_url`, sanitized through the
    // guard (host+path only). The commit SHA must survive (it is page content,
    // not a credential).
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-url-guard");
        let save_dir = temp_save_dir("timeline-url-guard");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        seed_timeline_frame_with_browser_url(
            &infra,
            &save_dir,
            "timeline-commit.jpg",
            "2026-05-17T10:00:00Z",
            Some("https://github.com/owner/repo/commit/9fceb02d8f1c3b4a5e6d7c8b9a0f1e2d3c4b5a6f"),
        )
        .await;

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_timeline(
            &config_dir,
            &infra,
            &[grant],
            BrokerTimelineRequest {
                from: "2026-05-17T00:00:00Z".to_string(),
                to: "2026-05-18T00:00:00Z".to_string(),
                limit: Some(5),
                app: None,
                window_title: None,
                url: None,
                url_regex: None,
            },
        )
        .await
        .expect("timeline should run")
        .expect("timeline should be authorized");

        let screen = response
            .intervals
            .iter()
            .find(|interval| interval.kind == "frame")
            .expect("screen interval should be present");
        assert_eq!(
            screen
                .context
                .as_ref()
                .and_then(|context| context.url.as_deref()),
            Some("github.com/owner/repo/commit/9fceb02d8f1c3b4a5e6d7c8b9a0f1e2d3c4b5a6f"),
            "interval url is the representative frame's guarded host+path (SHA preserved)"
        );
    });
}

#[test]
fn broker_timeline_interval_without_browser_url_keeps_context_but_no_url() {
    // A representative frame with no captured browser_url yields an interval
    // that still carries app/window context but no url.
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-no-url");
        let save_dir = temp_save_dir("timeline-no-url");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        seed_timeline_frame_with_browser_url(
            &infra,
            &save_dir,
            "timeline-no-url.jpg",
            "2026-05-17T10:00:00Z",
            None,
        )
        .await;

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_timeline(
            &config_dir,
            &infra,
            &[grant],
            BrokerTimelineRequest {
                from: "2026-05-17T00:00:00Z".to_string(),
                to: "2026-05-18T00:00:00Z".to_string(),
                limit: Some(5),
                app: None,
                window_title: None,
                url: None,
                url_regex: None,
            },
        )
        .await
        .expect("timeline should run")
        .expect("timeline should be authorized");

        let screen = response
            .intervals
            .iter()
            .find(|interval| interval.kind == "frame")
            .expect("screen interval should be present");
        let context = screen
            .context
            .as_ref()
            .expect("interval keeps app/window context");
        assert_eq!(context.app_name.as_deref(), Some("Google Chrome"));
        assert_eq!(
            context.url, None,
            "no captured browser_url means no interval url"
        );
    });
}

/// Insert a real frame carrying `browser_url` in its metadata snapshot, then a
/// `search_documents` frame row pointing at it under `group_key`. Returns the
/// inserted `(frame_id, search_documents.id)`. Inserting the search row directly
/// (rather than via the OCR/equivalent-reuse pipeline) lets a test put MULTIPLE
/// distinct frames into ONE timeline group with a controlled `id` ordering, to
/// pin down which frame the interval treats as representative.
async fn seed_grouped_frame_search_row(
    infra: &AppInfra,
    save_dir: &std::path::Path,
    file_name: &str,
    captured_at: &str,
    group_key: &str,
    browser_url: &str,
) -> (i64, i64) {
    let frame = infra
        .insert_frame(
            &NewFrame::new(
                "screen-session",
                save_dir.join(file_name).display().to_string(),
                captured_at,
            )
            .with_metadata_snapshot(capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.google.Chrome".to_string()),
                app_name: Some("Google Chrome".to_string()),
                window_title: Some("Pull Request".to_string()),
                window_id: None,
                browser_url: Some(browser_url.to_string()),
                display_id: Some(1),
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            }),
        )
        .await
        .expect("frame should insert");
    let document_id: i64 = sqlx::query_scalar(
        "INSERT INTO search_documents \
            (anchor_type, frame_id, absolute_start_at, absolute_end_at, session_id, \
             app_bundle_id, app_name, window_title, group_key, text_source_kind, body_text) \
         VALUES ('frame', ?, ?, ?, 'screen-session', \
             'com.google.Chrome', 'Google Chrome', 'Pull Request', ?, 'direct', 'grouped body') \
         RETURNING id",
    )
    .bind(frame.id)
    .bind(captured_at)
    .bind(captured_at)
    .bind(group_key)
    .fetch_one(infra.pool())
    .await
    .expect("search document should insert");
    (frame.id, document_id)
}

#[test]
fn broker_timeline_interval_url_is_deterministically_the_max_id_landing_frame() {
    // FIX #7 (deterministic representative frame): when a timeline GROUP spans
    // multiple frames (different frame_ids, different browser_urls), the
    // interval's guarded url MUST come from the MAX(id) (landing) frame, NOT an
    // arbitrary group member. The old query selected a bare `frame_id` alongside
    // MIN+two-MAX aggregates, where SQLite's row choice is documented-arbitrary;
    // the CTE+PK-join now pins it to the MAX(id) row.
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-deterministic-rep");
        let save_dir = temp_save_dir("timeline-deterministic-rep");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        let group_key = "frame:eq:test-group";
        // Two frames in the SAME group with DIFFERENT frame_ids + browser_urls.
        // The SECOND insert gets the larger autoincrement search_documents.id, so
        // it is the MAX(id) landing frame the interval must adopt.
        let (_earlier_frame, earlier_doc) = seed_grouped_frame_search_row(
            &infra,
            &save_dir,
            "timeline-earlier.jpg",
            "2026-05-17T10:00:00Z",
            group_key,
            "https://example.com/earlier-page",
        )
        .await;
        let (_landing_frame, landing_doc) = seed_grouped_frame_search_row(
            &infra,
            &save_dir,
            "timeline-landing.jpg",
            "2026-05-17T10:05:00Z",
            group_key,
            "https://example.com/landing-page",
        )
        .await;
        assert!(
            landing_doc > earlier_doc,
            "landing frame must hold the MAX(id) so it is the representative"
        );

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_timeline(
            &config_dir,
            &infra,
            &[grant],
            BrokerTimelineRequest {
                from: "2026-05-17T00:00:00Z".to_string(),
                to: "2026-05-18T00:00:00Z".to_string(),
                limit: Some(5),
                app: None,
                window_title: None,
                url: None,
                url_regex: None,
            },
        )
        .await
        .expect("timeline should run")
        .expect("timeline should be authorized");

        let screen = response
            .intervals
            .iter()
            .find(|interval| interval.kind == "frame")
            .expect("screen interval should be present");
        assert_eq!(
            screen
                .context
                .as_ref()
                .and_then(|context| context.url.as_deref()),
            Some("example.com/landing-page"),
            "interval url is deterministically the MAX(id) landing frame's guarded url"
        );
    });
}

#[test]
fn broker_timeline_batches_representative_snapshot_loads_preserving_urls() {
    // `broker_frame_timeline` loads every interval's representative-frame
    // snapshot in ONE batched query (`get_frame_metadata_snapshots`) instead of
    // a per-interval `get_frame` round-trip (the N+1 that, with `limit` clamped
    // to MAX_SEARCH_LIMIT=100, was up to 100 sequential round-trips on this
    // interactive broker path). The batching is a perf property; asserting it
    // via a global call counter is non-deterministic under the parallel test
    // harness, so this test pins the OBSERVABLE correctness the batching must
    // preserve — every group still surfaces with its guarded url resolved
    // through the single snapshot load — and relies on the batched
    // implementation for the perf win.
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-batch-snapshots");
        let save_dir = temp_save_dir("timeline-batch-snapshots");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        // Seed N distinct timeline groups (distinct group_key => distinct
        // interval => distinct representative frame), each carrying a browser_url
        // so every interval must resolve a guarded url through the batch load.
        const GROUPS: usize = 8;
        for i in 0..GROUPS {
            seed_grouped_frame_search_row(
                &infra,
                &save_dir,
                &format!("timeline-batch-{i}.jpg"),
                "2026-05-17T10:00:00Z",
                &format!("frame:eq:batch-group-{i}"),
                &format!("https://example.com/page-{i}"),
            )
            .await;
        }

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_timeline(
            &config_dir,
            &infra,
            &[grant],
            BrokerTimelineRequest {
                from: "2026-05-17T00:00:00Z".to_string(),
                to: "2026-05-18T00:00:00Z".to_string(),
                limit: Some(100),
                app: None,
                window_title: None,
                url: None,
                url_regex: None,
            },
        )
        .await
        .expect("timeline should run")
        .expect("timeline should be authorized");

        let screen_with_url = response
            .intervals
            .iter()
            .filter(|interval| interval.kind == "frame")
            .filter(|interval| {
                interval
                    .context
                    .as_ref()
                    .and_then(|context| context.url.as_deref())
                    .is_some()
            })
            .count();
        assert_eq!(
            screen_with_url, GROUPS,
            "every group's guarded url must resolve through the batched snapshot load"
        );
    });
}

#[test]
fn broker_timeline_without_context_filters_includes_frame_and_audio_intervals() {
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-all-sources");
        let save_dir = temp_save_dir("timeline-all-sources");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        let frame = infra
            .insert_frame(&NewFrame::new(
                "screen-session",
                save_dir.join("timeline-screen.jpg").display().to_string(),
                "2026-05-17T10:01:00Z",
            ))
            .await
            .expect("frame should insert");
        let job = infra
            .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
            .await
            .expect("OCR job should enqueue");
        let running = infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("OCR job should claim")
            .expect("OCR job should exist");
        infra
            .complete_processing_job(
                running.id,
                &ProcessingResultDraft::new().with_result_text("timeline body"),
            )
            .await
            .expect("OCR job should complete");

        infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                save_dir.join("audio.m4a").display().to_string(),
                "2026-05-17T10:00:00Z",
                "2026-05-17T10:00:30Z",
            ))
            .await
            .expect("audio segment should insert");
        infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::SystemAudio,
                "system-session",
                1,
                save_dir.join("system.m4a").display().to_string(),
                "2026-05-17T09:59:00Z",
                "2026-05-17T09:59:30Z",
            ))
            .await
            .expect("system audio segment should insert");

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_timeline(
            &config_dir,
            &infra,
            &[grant],
            BrokerTimelineRequest {
                from: "2026-05-17T00:00:00Z".to_string(),
                to: "2026-05-18T00:00:00Z".to_string(),
                limit: Some(5),
                app: None,
                window_title: None,
                url: None,
                url_regex: None,
            },
        )
        .await
        .expect("timeline should run")
        .expect("timeline should be authorized");

        // Newest first, and one vocabulary with `search`: the user's own voice
        // must never read the same as a video playing through the speakers.
        assert_eq!(
            response
                .intervals
                .iter()
                .map(|interval| interval.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["frame", "audio_microphone", "audio_system"]
        );
    });
}

#[test]
fn broker_timeline_audio_interval_opaque_id_round_trips_through_show_text() {
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-audio-opaque");
        let save_dir = temp_save_dir("timeline-audio-opaque");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        let segment = infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                save_dir.join("audio.m4a").display().to_string(),
                "2026-05-17T10:00:00Z",
                "2026-05-17T10:00:30Z",
            ))
            .await
            .expect("audio segment should insert");
        let job = infra
            .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                segment.id,
            ))
            .await
            .expect("transcription job should enqueue");
        let running = infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("transcription job should claim")
            .expect("transcription job should exist");
        infra
            .complete_processing_job(
                running.id,
                &ProcessingResultDraft::new().with_result_text("timeline transcript"),
            )
            .await
            .expect("transcription job should complete");

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_timeline(
            &config_dir,
            &infra,
            &[grant.clone()],
            BrokerTimelineRequest {
                from: "2026-05-17T00:00:00Z".to_string(),
                to: "2026-05-18T00:00:00Z".to_string(),
                limit: Some(5),
                app: None,
                window_title: None,
                url: None,
                url_regex: None,
            },
        )
        .await
        .expect("timeline should run")
        .expect("timeline should be authorized");

        let opaque_id = response
            .intervals
            .iter()
            .find(|interval| interval.kind.starts_with("audio"))
            .expect("audio interval should exist")
            .opaque_id
            .clone()
            .expect("audio interval should carry an opaque id");

        let text = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run")
            .expect("timeline opaque id should be authorized");
        assert_eq!(text.text, "timeline transcript");
    });
}

#[test]
fn broker_timeline_screen_interval_opaque_id_round_trips_through_show_text() {
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-screen-opaque");
        let save_dir = temp_save_dir("timeline-screen-opaque");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        seed_timeline_frame_with_browser_url(
            &infra,
            &save_dir,
            "timeline-screen-opaque.jpg",
            "2026-05-17T10:00:00Z",
            None,
        )
        .await;

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_timeline(
            &config_dir,
            &infra,
            &[grant.clone()],
            BrokerTimelineRequest {
                from: "2026-05-17T00:00:00Z".to_string(),
                to: "2026-05-18T00:00:00Z".to_string(),
                limit: Some(5),
                app: None,
                window_title: None,
                url: None,
                url_regex: None,
            },
        )
        .await
        .expect("timeline should run")
        .expect("timeline should be authorized");

        let opaque_id = response
            .intervals
            .iter()
            .find(|interval| interval.kind == "frame")
            .expect("screen interval should exist")
            .opaque_id
            .clone()
            .expect("screen interval should carry an opaque id");

        let text = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run")
            .expect("timeline opaque id should be authorized");
        assert_eq!(text.text, "timeline body");
    });
}

#[test]
fn broker_timeline_omits_opaque_id_when_no_representative_frame() {
    // An interval with no representative frame has nothing to follow up on: the
    // field must be ABSENT from the wire, never a literal null (the shape the
    // deleted `reason` field emitted on every interval ever sent). Asserted on
    // the struct rather than through a seeded row because `search_documents`
    // CHECKs `anchor_type = 'frame' AND frame_id IS NOT NULL` — the None arm is
    // reachable only from the nullable column type, not from legal data.
    let interval = BrokerTimelineInterval {
        kind: "frame".to_string(),
        started_at: "2026-05-17T10:00:00Z".to_string(),
        ended_at: Some("2026-05-17T10:00:30Z".to_string()),
        opaque_id: None,
        context: None,
    };
    let json = serde_json::to_string(&interval).expect("interval should serialize");
    assert!(
        !json.contains("opaqueId"),
        "absent representative frame must omit the field, not emit null: {json}"
    );
}

#[test]
fn broker_show_text_reports_the_split_audio_kind_per_source() {
    run_async_test(async {
        let config_dir = temp_config_dir("show-text-audio-kind");
        let save_dir = temp_save_dir("show-text-audio-kind");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let now = now_unix_ms();
        let started_at = format_unix_ms(now.saturating_sub(2 * 60 * 60 * 1000));
        let ended_at = format_unix_ms(now);
        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");

        for (index, source_kind, expected) in [
            (1, AudioSegmentSourceKind::Microphone, "audio_microphone"),
            (2, AudioSegmentSourceKind::SystemAudio, "audio_system"),
        ] {
            let segment = infra
                .upsert_audio_segment(&NewAudioSegment::new(
                    source_kind,
                    "kind-session",
                    index,
                    save_dir
                        .join(format!("audio-{index}.m4a"))
                        .display()
                        .to_string(),
                    started_at.clone(),
                    ended_at.clone(),
                ))
                .await
                .expect("segment should insert");
            let job = infra
                .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                    segment.id,
                ))
                .await
                .expect("job should enqueue");
            let running = infra
                .claim_queued_processing_job(job.id)
                .await
                .expect("job should claim")
                .expect("job should exist");
            infra
                .complete_processing_job(
                    running.id,
                    &ProcessingResultDraft::new().with_result_text("kinded transcript"),
                )
                .await
                .expect("job should complete");
            let opaque_id =
                encode_signed_opaque_id("audio", segment.id, Some(&grant.id), &secret);

            let response =
                broker_show_text(&config_dir, &infra, std::slice::from_ref(&grant), &opaque_id)
                    .await
                    .expect("show text should run")
                    .expect("audio should be authorized");

            // The opaque id's prefix is `audio` for both: the reported kind must
            // come from the segment, or `show-text` contradicts `search`.
            assert_eq!(response.kind, expected);
        }
    });
}

#[test]
fn broker_search_splits_audio_kind_by_segment_source() {
    let secret = &[7u8; 32];
    for (source_kind, expected) in [
        (AudioSegmentSourceKind::Microphone, "audio_microphone"),
        (AudioSegmentSourceKind::SystemAudio, "audio_system"),
    ] {
        let mut response = two_frame_one_audio_response();
        response.frames.clear();
        response.audio[0].audio_segment.source_kind = source_kind;
        let mapped = map_search_response(response, 5, None, Some("grant-1"), secret);
        assert_eq!(mapped.results[0].kind, expected);
    }
}

#[test]
fn broker_show_text_authorizes_audio_by_segment_overlap() {
    run_async_test(async {
        let config_dir = temp_config_dir("audio-overlap");
        let save_dir = temp_save_dir("audio-overlap");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let now = now_unix_ms();
        let started_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
        let ended_at = format_unix_ms(now);
        let segment = infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                save_dir.join("audio.m4a").display().to_string(),
                started_at,
                ended_at,
            ))
            .await
            .expect("segment should insert");
        let job = infra
            .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                segment.id,
            ))
            .await
            .expect("job should enqueue");
        let running = infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("job should claim")
            .expect("job should exist");
        infra
            .complete_processing_job(
                running.id,
                &ProcessingResultDraft::new().with_result_text("overlapping transcript"),
            )
            .await
            .expect("job should complete");
        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id = encode_signed_opaque_id("audio", segment.id, Some(&grant.id), &secret);

        let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run")
            .expect("overlapping audio should be authorized");

        assert_eq!(response.text, "overlapping transcript");
        assert!(
            response.speakers.is_empty(),
            "audio without speaker analysis must carry no speakers"
        );
    });
}

#[test]
fn ask_ai_show_text_authorizes_all_retained_without_persisted_grant() {
    run_async_test(async {
        let config_dir = temp_config_dir("ask-ai-show-text");
        let save_dir = temp_save_dir("ask-ai-show-text");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        write_recording_settings(&config_dir, &save_dir);
        let now = now_unix_ms();
        let started_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
        let ended_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
        let segment = infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                save_dir.join("audio.m4a").display().to_string(),
                started_at,
                ended_at,
            ))
            .await
            .expect("segment should insert");
        let job = infra
            .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
                segment.id,
            ))
            .await
            .expect("job should enqueue");
        let running = infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("job should claim")
            .expect("job should exist");
        infra
            .complete_processing_job(
                running.id,
                &ProcessingResultDraft::new().with_result_text("all retained transcript"),
            )
            .await
            .expect("job should complete");

        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id =
            encode_signed_opaque_id("audio", segment.id, Some(ASK_AI_BROKER_GRANT_ID), &secret);

        let access = BrokeredCaptureAccess::from_config_dir(config_dir.clone());
        let identity =
            BrokerClientIdentity::new("PI", BrokerClientIdentitySource::Inferred).unwrap();

        let response = access
            .execute_for_ask_ai(identity, BrokeredCaptureRequest::ShowText { opaque_id })
            .await
            .unwrap();

        match response {
            BrokeredCaptureResponse::ShowText(show_text) => {
                assert_eq!(show_text.text, "all retained transcript");
            }
            other => panic!("expected ShowText response, got {other:?}"),
        }

        assert!(load_grants(&config_dir).unwrap().grants.is_empty());

        let audit = load_audit_events(&config_dir).unwrap();
        assert_eq!(audit.events.len(), 1);
        let event = &audit.events[0];
        assert_eq!(event.scope_class, "all_retained_history");
        assert_eq!(event.grant_id, Some(ASK_AI_BROKER_GRANT_ID.to_string()));
        assert_eq!(event.command_type, "show_text");
        assert_eq!(event.tool_identity, "PI");
    });
}

#[test]
fn ask_ai_timeline_reaches_all_retained_history() {
    run_async_test(async {
        let config_dir = temp_config_dir("ask-ai-timeline");
        let save_dir = temp_save_dir("ask-ai-timeline");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        write_recording_settings(&config_dir, &save_dir);
        let now = now_unix_ms();
        let started_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
        let ended_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
        infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                save_dir.join("audio.m4a").display().to_string(),
                started_at,
                ended_at,
            ))
            .await
            .expect("segment should insert");

        let access = BrokeredCaptureAccess::from_config_dir(config_dir.clone());
        let identity =
            BrokerClientIdentity::new("PI", BrokerClientIdentitySource::Inferred).unwrap();

        let from = format_unix_ms(now.saturating_sub(3 * 24 * 60 * 60 * 1000));
        let to = format_unix_ms(now);
        let response = access
            .execute_for_ask_ai(
                identity,
                BrokeredCaptureRequest::Timeline(BrokerTimelineRequest {
                    from,
                    to,
                    limit: Some(50),
                    app: None,
                    window_title: None,
                    url: None,
                    url_regex: None,
                }),
            )
            .await
            .unwrap();

        match response {
            BrokeredCaptureResponse::Timeline(timeline) => {
                assert!(!timeline.intervals.is_empty());
                assert!(timeline
                    .intervals
                    .iter()
                    .any(|interval| interval.kind == "audio_microphone"));
            }
            other => panic!("expected Timeline response, got {other:?}"),
        }

        assert!(load_grants(&config_dir).unwrap().grants.is_empty());
    });
}

#[test]
fn ask_ai_rejects_open_in_mnema_as_non_data_tool() {
    run_async_test(async {
        let access =
            BrokeredCaptureAccess::from_config_dir(temp_config_dir("ask-ai-open").clone());
        let identity =
            BrokerClientIdentity::new("PI", BrokerClientIdentitySource::Inferred).unwrap();

        let response = access
            .execute_for_ask_ai(
                identity,
                BrokeredCaptureRequest::OpenInMnema {
                    opaque_id: "anything".into(),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            response,
            BrokeredCaptureResponse::Error(BrokerErrorResponse::authorization_required())
        );
    });
}

#[test]
fn ask_ai_rejects_open_captured_url_as_non_data_tool() {
    run_async_test(async {
        let access =
            BrokeredCaptureAccess::from_config_dir(temp_config_dir("ask-ai-open-url").clone());
        let identity =
            BrokerClientIdentity::new("PI", BrokerClientIdentitySource::Inferred).unwrap();

        let response = access
            .execute_for_ask_ai(
                identity,
                BrokeredCaptureRequest::OpenCapturedUrl {
                    opaque_id: "anything".into(),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            response,
            BrokeredCaptureResponse::Error(BrokerErrorResponse::authorization_required())
        );
    });
}

#[test]
fn execute_rejects_open_captured_url_universally() {
    // SECURITY (the core of FIX #1): the external `execute`/`execute_for_identity`
    // path must NEVER open a raw captured URL — even for an authorized, in-scope
    // FRAME result whose `browser_url` is a perfectly openable http(s) URL. A
    // grant-holding CLI/agent navigating the user's authenticated browser to an
    // in-scope captured URL is a CSRF/replay primitive (ADR 0038). The broker
    // must reject with `authorization_required`, never reaching any OS opener.
    run_async_test(async {
        let config_dir = temp_config_dir("execute-rejects-open-url");
        let save_dir = temp_save_dir("execute-rejects-open-url");
        write_recording_settings(&config_dir, &save_dir);
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        // Seed a real, in-scope frame WITH a valid https browser_url, so the
        // only reason for rejection is the universal broker policy (not a
        // missing/invalid/non-http URL).
        let frame = infra
            .insert_frame(
                &NewFrame::new(
                    "screen-session",
                    save_dir.join("open-url-frame.jpg").display().to_string(),
                    &format_unix_ms(now_unix_ms().saturating_sub(60 * 1000)),
                )
                .with_metadata_snapshot(capture_metadata::FrameMetadataSnapshot {
                    app_bundle_id: Some("com.google.Chrome".to_string()),
                    app_name: Some("Google Chrome".to_string()),
                    window_title: Some("Example".to_string()),
                    window_id: None,
                    browser_url: Some("https://example.com/path".to_string()),
                    display_id: Some(1),
                    metadata_redaction_reason: None,
                    metadata_redaction_source_id: None,
                }),
            )
            .await
            .expect("frame should insert");
        // Grant + identity must share a normalized label so `execute` resolves
        // active grants for the caller before the handler runs.
        let grant = create_grant(
            &config_dir,
            "mnema-cli",
            1,
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id = encode_signed_opaque_id("frame", frame.id, Some(&grant.id), &secret);

        let access = BrokeredCaptureAccess::from_config_dir(config_dir.clone());
        let response = access
            .execute(
                "mnema-cli",
                BrokeredCaptureRequest::OpenCapturedUrl { opaque_id },
            )
            .await
            .expect("open-captured-url should run");

        assert_eq!(
            response,
            BrokeredCaptureResponse::Error(BrokerErrorResponse::authorization_required()),
            "the broker must never open a captured URL, even for an authorized http(s) frame"
        );
    });
}

#[test]
fn broker_show_text_resolves_equivalent_reuse_frame_text() {
    run_async_test(async {
        let config_dir = temp_config_dir("equivalent-reuse-show-text");
        let save_dir = temp_save_dir("equivalent-reuse-show-text");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let equivalence = crate::FrameEquivalence {
            hint: Some("same-screen".to_string()),
            proof: Some(vec![9; 1024]),
            version: Some(1),
            status: Some(crate::FrameEquivalenceStatus::Ready),
            error: None,
        };
        let first = infra
            .capture_frame(
                &NewFrame::new(
                    "screen-session",
                    "/tmp/broker-show-text-source.jpg",
                    "2026-05-17T10:00:00Z",
                )
                .with_equivalence(equivalence.clone()),
                None,
            )
            .await
            .expect("first frame should capture");
        let job = first.job.expect("first frame should enqueue OCR");
        let running = infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("job should claim")
            .expect("job should exist");
        infra
            .complete_processing_job(
                running.id,
                &ProcessingResultDraft::new().with_result_text("reused frame text"),
            )
            .await
            .expect("job should complete");

        let second = infra
            .capture_frame(
                &NewFrame::new(
                    "screen-session",
                    "/tmp/broker-show-text-duplicate.jpg",
                    "2026-05-17T10:00:02Z",
                )
                .with_equivalence(equivalence),
                None,
            )
            .await
            .expect("second frame should capture");
        assert!(second.job.is_none());
        assert!(infra
            .list_processing_results_for_subject(&ProcessingSubject::frame(second.frame.id))
            .await
            .expect("results should list")
            .is_empty());

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id =
            encode_signed_opaque_id("frame", second.frame.id, Some(&grant.id), &secret);

        let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run")
            .expect("equivalent-reuse frame should resolve source text");

        assert_eq!(response.text, "reused frame text");
    });
}

#[test]
fn broker_show_text_rejects_equivalent_reuse_source_outside_scope() {
    run_async_test(async {
        let config_dir = temp_config_dir("equivalent-reuse-show-text-outside-scope");
        let save_dir = temp_save_dir("equivalent-reuse-show-text-outside-scope");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let equivalence = crate::FrameEquivalence {
            hint: Some("same-screen".to_string()),
            proof: Some(vec![10; 1024]),
            version: Some(1),
            status: Some(crate::FrameEquivalenceStatus::Ready),
            error: None,
        };
        let now = now_unix_ms();
        let source_captured_at = format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000));
        let duplicate_captured_at = format_unix_ms(now.saturating_sub(60 * 1000));
        let first = infra
            .capture_frame(
                &NewFrame::new(
                    "screen-session",
                    "/tmp/broker-show-text-source-outside-scope.jpg",
                    &source_captured_at,
                )
                .with_equivalence(equivalence.clone()),
                None,
            )
            .await
            .expect("first frame should capture");
        let job = first.job.expect("first frame should enqueue OCR");
        let running = infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("job should claim")
            .expect("job should exist");
        infra
            .complete_processing_job(
                running.id,
                &ProcessingResultDraft::new().with_result_text("out-of-scope reused text"),
            )
            .await
            .expect("job should complete");

        let second = infra
            .capture_frame(
                &NewFrame::new(
                    "screen-session",
                    "/tmp/broker-show-text-duplicate-in-scope.jpg",
                    &duplicate_captured_at,
                )
                .with_equivalence(equivalence),
                None,
            )
            .await
            .expect("second frame should capture");
        assert!(second.job.is_none());

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id =
            encode_signed_opaque_id("frame", second.frame.id, Some(&grant.id), &secret);

        let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run");

        assert_eq!(response, Err(outside_scope_error()));
    });
}

#[test]
fn broker_rejects_unsigned_opaque_ids_for_authorized_commands() {
    run_async_test(async {
        let config_dir = temp_config_dir("unsigned-opaque");
        let save_dir = temp_save_dir("unsigned-opaque");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_authorize_opaque_reference(&config_dir, &infra, &[grant], "f1")
            .await
            .expect("authorization should run");

        assert_eq!(response, Err(invalid_opaque_id_error()));
    });
}

#[test]
fn active_opaque_authorization_rejects_revoked_grant_replay() {
    run_async_test(async {
        let config_dir = temp_config_dir("revoked-opaque-replay");
        let save_dir = temp_save_dir("revoked-opaque-replay");
        write_recording_settings(&config_dir, &save_dir);
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let frame = infra
            .capture_frame(
                &NewFrame::new(
                    "screen-session",
                    "/tmp/broker-revoked-replay.jpg",
                    &format_unix_ms(now_unix_ms()),
                ),
                None,
            )
            .await
            .expect("frame should capture")
            .frame;
        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id = encode_signed_opaque_id("frame", frame.id, Some(&grant.id), &secret);

        assert!(revoke_grant(&config_dir, &grant.id).expect("grant should revoke"));

        let response = authorize_active_opaque_capture_reference(&config_dir, &opaque_id)
            .await
            .expect("authorization should run");

        assert_eq!(response, None);
    });
}

#[test]
fn active_opaque_authorization_rejects_ids_for_different_active_grant() {
    run_async_test(async {
        let config_dir = temp_config_dir("cross-grant-opaque-replay");
        let save_dir = temp_save_dir("cross-grant-opaque-replay");
        write_recording_settings(&config_dir, &save_dir);
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let frame = infra
            .capture_frame(
                &NewFrame::new(
                    "screen-session",
                    "/tmp/broker-cross-grant-replay.jpg",
                    &format_unix_ms(now_unix_ms()),
                ),
                None,
            )
            .await
            .expect("frame should capture")
            .frame;
        let original_grant = create_grant(
            &config_dir,
            "Original agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let _other_grant = create_grant(
            &config_dir,
            "Other agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("other grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id =
            encode_signed_opaque_id("frame", frame.id, Some(&original_grant.id), &secret);

        assert!(revoke_grant(&config_dir, &original_grant.id).expect("grant should revoke"));

        let response = authorize_active_opaque_capture_reference(&config_dir, &opaque_id)
            .await
            .expect("authorization should run");

        assert_eq!(response, None);
    });
}

/// `limit: 0` is reachable from the wire (the MCP `mnema_search` tool passes
/// `limit` straight through with no lower bound). A zero page can never make
/// progress, so the walk terminates on its FIRST page with `next_cursor:
/// None` — which the response contract defines as "this page exhausted the
/// matches". A caller that honours that contract concludes the query has no
/// matches at all, silently dropping the entire result set.
#[test]
fn broker_search_with_zero_limit_does_not_claim_the_walk_is_exhausted() {
    run_async_test(async {
        let config_dir = temp_config_dir("search-zero-limit");
        let save_dir = temp_save_dir("search-zero-limit");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        let frame = infra
            .insert_frame(&NewFrame::new(
                "screen-session",
                save_dir.join("zero-limit.jpg").display().to_string(),
                "2026-05-17T10:00:00Z",
            ))
            .await
            .expect("frame should insert");
        let job = infra
            .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
            .await
            .expect("OCR job should enqueue");
        let running = infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("OCR job should claim")
            .expect("OCR job should exist");
        infra
            .complete_processing_job(
                running.id,
                &ProcessingResultDraft::new().with_result_text("zerolimit body text"),
            )
            .await
            .expect("OCR job should complete");

        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let search = |limit: u32| {
            let infra = &infra;
            let grant = grant.clone();
            let config_dir = config_dir.clone();
            async move {
                broker_search(
                    &config_dir,
                    infra,
                    &[grant],
                    BrokerSearchRequest {
                        query: "zerolimit".to_string(),
                        from: None,
                        to: None,
                        limit: Some(limit),
                        app: None,
                        window_title: None,
                        url: None,
                        url_regex: None,
                        cursor: None,
                    },
                )
                .await
                .expect("search should run")
                .expect("search should be authorized")
            }
        };

        // The match really is there.
        assert_eq!(search(5).await.results.len(), 1);

        let zero = search(0).await;
        assert!(
            !zero.results.is_empty() || zero.next_cursor.is_some(),
            "limit 0 reported an exhausted walk over a non-empty match set: {zero:?}"
        );
    });
}

fn speaker_embedding_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

fn speaker_cluster(
    provider_cluster_id: &str,
    embedding: &[f32],
) -> speaker_analysis::SpeakerCluster {
    speaker_analysis::SpeakerCluster {
        provider_cluster_id: provider_cluster_id.to_string(),
        stable_label: format!("Unknown {provider_cluster_id}"),
        embedding: speaker_embedding_bytes(embedding),
        embedding_model_id: "voice-model".to_string(),
        suggestion: None,
    }
}

fn speaker_turn(
    provider_cluster_id: &str,
    start_ms: u64,
    end_ms: u64,
) -> speaker_analysis::SpeakerTurn {
    speaker_analysis::SpeakerTurn {
        provider_cluster_id: provider_cluster_id.to_string(),
        start_ms,
        end_ms,
        transcript_text: None,
        overlaps: false,
    }
}

fn speaker_analysis_output(
    session_id: &str,
    audio_segment_id: i64,
    clusters: Vec<speaker_analysis::SpeakerCluster>,
    turns: Vec<speaker_analysis::SpeakerTurn>,
) -> speaker_analysis::SpeakerAnalysisOutput {
    speaker_analysis::SpeakerAnalysisOutput {
        clusters,
        turns,
        metadata: speaker_analysis::SpeakerAnalysisMetadata {
            provider: "mock_speaker".to_string(),
            model_id: Some("voice-model".to_string()),
            session_id: session_id.to_string(),
            audio_segment_id,
            provenance: Default::default(),
        },
        provider_version: None,
    }
}

/// Seeds an in-scope audio segment whose transcript the broker will serve and
/// returns the segment id plus the grant-signed opaque id an agent presents.
async fn seed_brokered_audio_segment(
    config_dir: &Path,
    save_dir: &Path,
    infra: &AppInfra,
    session_id: &str,
    transcript: &str,
) -> (i64, BrokerGrant, String) {
    let now = now_unix_ms();
    let segment = infra
        .upsert_audio_segment(&NewAudioSegment::new(
            AudioSegmentSourceKind::Microphone,
            session_id,
            1,
            save_dir.join("audio.m4a").display().to_string(),
            format_unix_ms(now.saturating_sub(60 * 60 * 1000)),
            format_unix_ms(now),
        ))
        .await
        .expect("segment should insert");
    let job = infra
        .enqueue_processing_job(&ProcessingJobDraft::for_audio_segment_transcription(
            segment.id,
        ))
        .await
        .expect("transcription job should enqueue");
    let running = infra
        .claim_queued_processing_job(job.id)
        .await
        .expect("transcription job should claim")
        .expect("transcription job should exist");
    infra
        .complete_processing_job(
            running.id,
            &ProcessingResultDraft::new().with_result_text(transcript),
        )
        .await
        .expect("transcription should complete");
    let grant = create_grant(
        config_dir,
        "Local agent",
        1,
        BrokerGrantScope::RecentDays { days: 1 },
    )
    .expect("grant should create");
    let secret = load_or_create_opaque_secret(config_dir).expect("secret should load");
    let opaque_id = encode_signed_opaque_id("audio", segment.id, Some(&grant.id), &secret);
    (segment.id, grant, opaque_id)
}

/// Persists speaker clusters/turns through the real speaker-analysis job
/// completion path (the only writer of `recording_speaker_clusters`).
async fn complete_speaker_analysis(
    infra: &AppInfra,
    audio_segment_id: i64,
    output: speaker_analysis::SpeakerAnalysisOutput,
) {
    let job = infra
        .enqueue_processing_job(
            &ProcessingJobDraft::for_audio_segment_speaker_analysis(audio_segment_id)
                .with_payload_json(
                    serde_json::to_string(&crate::SpeakerAnalysisJobPayload::new(
                        "mock_speaker",
                        Some("voice-model".to_string()),
                    ))
                    .expect("payload should encode"),
                ),
        )
        .await
        .expect("speaker analysis job should enqueue");
    let running = infra
        .claim_queued_processing_job(job.id)
        .await
        .expect("speaker analysis job should claim")
        .expect("speaker analysis job should exist");
    infra
        .complete_processing_job(
            running.id,
            &ProcessingResultDraft::new().with_structured_payload_json(
                serde_json::to_string(&output).expect("output should encode"),
            ),
        )
        .await
        .expect("speaker analysis should complete");
}

/// `speakers` promises "ordered by first turn". The EARLIER turn here belongs
/// to the SECOND cluster, so an ordering that fell back to cluster/insertion
/// order (or dropped the `ORDER BY start_ms` in the turn query) flips the
/// list and hands the agent the wrong voice as the one who spoke first.
#[test]
fn broker_show_text_lists_audio_speakers_in_first_turn_order() {
    run_async_test(async {
        let config_dir = temp_config_dir("speakers-turn-order");
        let save_dir = temp_save_dir("speakers-turn-order");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let (segment_id, grant, opaque_id) = seed_brokered_audio_segment(
            &config_dir,
            &save_dir,
            &infra,
            "turn-order-session",
            "two voices",
        )
        .await;
        let ada = infra
            .create_person_profile("Ada", None)
            .await
            .expect("person profile should insert");
        complete_speaker_analysis(
            &infra,
            segment_id,
            speaker_analysis_output(
                "turn-order-session",
                segment_id,
                vec![
                    speaker_cluster("speaker_00", &[1.0, 0.0]),
                    speaker_cluster("speaker_01", &[0.0, 1.0]),
                ],
                vec![
                    speaker_turn("speaker_00", 5_000, 6_000),
                    speaker_turn("speaker_01", 1_000, 2_000),
                ],
            ),
        )
        .await;
        let clusters = infra
            .list_speaker_clusters_for_session("turn-order-session")
            .await
            .expect("clusters should list");
        let later_cluster = clusters
            .iter()
            .find(|cluster| cluster.provider_cluster_id.ends_with("speaker_01"))
            .expect("the second cluster should exist");
        infra
            .link_speaker_cluster_to_person(later_cluster.id, ada.id, false)
            .await
            .expect("cluster should link");

        let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run")
            .expect("audio should be authorized");

        assert_eq!(
            response.speakers,
            vec![
                BrokerSpeaker {
                    name: Some("Ada".to_string()),
                    attribution: "assigned".to_string(),
                    confidence: None,
                },
                BrokerSpeaker {
                    name: None,
                    attribution: "unknown".to_string(),
                    confidence: None,
                },
            ],
            "speakers must follow first-turn order, not cluster order"
        );
    });
}

/// The subject-type gate: a frame result carries no speakers, and the key is
/// absent from the wire entirely so an older CLI/MCP client still parses it.
#[test]
fn broker_show_text_for_a_frame_carries_no_speakers() {
    run_async_test(async {
        let config_dir = temp_config_dir("frame-no-speakers");
        let save_dir = temp_save_dir("frame-no-speakers");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let frame = infra
            .insert_frame(&NewFrame::new(
                "screen-session",
                save_dir.join("frame.jpg").display().to_string(),
                &format_unix_ms(now_unix_ms().saturating_sub(60 * 1000)),
            ))
            .await
            .expect("frame should insert");
        let job = infra
            .enqueue_processing_job(&ProcessingJobDraft::for_frame_ocr(frame.id))
            .await
            .expect("OCR job should enqueue");
        let running = infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("OCR job should claim")
            .expect("OCR job should exist");
        infra
            .complete_processing_job(
                running.id,
                &ProcessingResultDraft::new().with_result_text("frame body"),
            )
            .await
            .expect("OCR job should complete");
        let grant = create_grant(
            &config_dir,
            "Local agent",
            1,
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id = encode_signed_opaque_id("frame", frame.id, Some(&grant.id), &secret);

        let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run")
            .expect("frame should be authorized");

        assert!(response.speakers.is_empty(), "frames have no speakers");
        let json = serde_json::to_value(&response).expect("response should serialize");
        assert!(
            json.get("speakers").is_none(),
            "the speakers key must stay off frame results: {json}"
        );
    });
}

/// Confirming a recognition and then unlinking the person used to leave a STALE
/// `recognition_person_id` on the cluster next to the fresh per-cluster
/// rejection. The unlink now clears it, so the broker reads the row straight;
/// this pins the end result at the wire — a name the user took back never
/// reaches an external agent.
#[test]
fn broker_show_text_omits_a_recognition_the_user_rejected_for_that_cluster() {
    run_async_test(async {
        let config_dir = temp_config_dir("speakers-rejected-recognition");
        let save_dir = temp_save_dir("speakers-rejected-recognition");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let (segment_id, grant, opaque_id) = seed_brokered_audio_segment(
            &config_dir,
            &save_dir,
            &infra,
            "rejected-session",
            "a private conversation",
        )
        .await;
        let person = infra
            .create_person_profile("Ada", None)
            .await
            .expect("person profile should insert");
        let mut output = speaker_analysis_output(
            "rejected-session",
            segment_id,
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        );
        output.clusters[0].suggestion =
            Some(speaker_analysis::SpeakerRecognitionSuggestion {
                person_id: person.id,
                display_name: "Ada".to_string(),
                confidence: speaker_analysis::RecognitionConfidence::High,
                score: 0.91,
            });
        complete_speaker_analysis(&infra, segment_id, output).await;
        let cluster = infra
            .list_speaker_clusters_for_session("rejected-session")
            .await
            .expect("clusters should list")
            .into_iter()
            .next()
            .expect("cluster should exist");
        assert_eq!(cluster.suggested_person_id, Some(person.id));
        infra
            .confirm_speaker_recognition_suggestion(cluster.id, false)
            .await
            .expect("suggestion should confirm");
        let unlinked = infra
            .unlink_speaker_cluster_from_person(cluster.id)
            .await
            .expect("cluster should unlink");
        assert_eq!(unlinked.person_id, None);
        assert_eq!(unlinked.suggested_person_id, None);

        let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run")
            .expect("audio should be authorized");

        assert_eq!(
            response.speakers,
            vec![BrokerSpeaker {
                name: None,
                attribution: "unknown".to_string(),
                confidence: None,
            }]
        );
        let json = serde_json::to_string(&response).expect("response should serialize");
        assert!(
            !json.contains("Ada"),
            "a rejected recognition must never reach an agent: {json}"
        );
    });
}
