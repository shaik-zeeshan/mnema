use super::*;
// Test-only lexical primitives (the non-test code references the other aliases
// at module scope); kept under their historical `recall_*` names.
use crate::lexical::{idf_weight as recall_idf_weight, stem as recall_stem};
use crate::{
    AppInfra, NewAudioSegment, NewFrame, ProcessingJobDraft, ProcessingResultDraft,
    SearchCaptureRefinements, SearchCaptureResponse, SearchDateRangeOrigin,
    SearchDateRangeRefinement,
};

/// Test-only shorthand for the production upsert. Permissions no longer carry a
/// duration, so this is `label + scope` and nothing else.
fn create_grant(config_dir: &Path, label: &str, scope: BrokerGrantScope) -> Result<BrokerGrant> {
    let identity = BrokerClientIdentity::new(label, BrokerClientIdentitySource::Explicit)?;
    Ok(upsert_grant_for_identity(config_dir, identity, scope)?.grant)
}

fn stored_grants(config_dir: &Path) -> Vec<BrokerGrant> {
    load_grants(config_dir).expect("grants should load").grants
}

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

fn execute_request(config_dir: &Path, request: BrokeredCaptureRequest) -> BrokeredCaptureResponse {
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
    let recalled = select_relevant_conclusions(
        &conclusions,
        &tokens,
        MAX_RECALL_CONTEXT_LIMIT as usize,
        true,
    );
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
    assert_eq!(
        recalled.len(),
        1,
        "only the whole-word match should survive"
    );
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
    assert!(
        recalled.is_empty(),
        "over-suppression by design: {recalled:?}"
    );
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
            "mnema CLI",
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
            "mnema CLI",
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
            response
                .activities
                .iter()
                .all(|a| { !crate::user_context::guardrail::is_sensitive(&a.title, &a.summary) }),
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
            "mnema CLI",
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
fn capture_request_without_active_grants_is_denied_and_audited_as_denied() {
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
            speaker: None,
        }),
    );

    assert_eq!(
        response,
        BrokeredCaptureResponse::Error(BrokerErrorResponse::authorization_required())
    );
    // A permission log that records only successes cannot answer "did anything
    // try and get turned away", which is the whole point of the activity list.
    let audit = load_audit_events(&config_dir).unwrap();
    assert_eq!(audit.events.len(), 1);
    assert_eq!(audit.events[0].command_type, "search");
    assert_eq!(audit.events[0].outcome.as_deref(), Some("denied"));
    assert_eq!(audit.events[0].scope_class, "none");
    assert_eq!(audit.events[0].grant_id, None);
}

#[test]
fn invalid_open_request_is_shaped_and_audited_by_brokered_capture_access() {
    let config_dir = temp_config_dir("invalid-open");
    create_grant(&config_dir, "mnema CLI", BrokerGrantScope::LAST_DAY).unwrap();

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
    assert_eq!(audit.events[0].outcome.as_deref(), Some("scope_rejected"));
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
    let mapped = map_search_response(response, 2, None, Some("grant-1"), secret, HashMap::new());

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
            speaker: None,
        },
        residual_query: "target".to_string(),
        parse_errors: Vec::new(),
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
    let mapped = map_search_response(response, 2, None, Some("grant-1"), secret, HashMap::new());
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
    let mapped = map_search_response(response, 2, None, Some("grant-1"), secret, HashMap::new());
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
        HashMap::new(),
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
        HashMap::new(),
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
        HashMap::new(),
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

    let mapped = map_search_response(response, 5, None, Some("grant-1"), secret, HashMap::new());

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

    let mapped = map_search_response(response, 5, None, Some("grant-1"), secret, HashMap::new());

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

    let mapped = map_search_response(response, 5, None, Some("grant-1"), secret, HashMap::new());

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

    let mapped = map_search_response(response, 5, None, Some("grant-1"), secret, HashMap::new());

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
            "mnema CLI",
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
                        speaker: None,
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
            "mnema CLI",
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
                speaker: None,
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

        let snapshot = |title: &str, url: &str| capture_metadata::FrameMetadataSnapshot {
            app_bundle_id: Some("com.google.Chrome".to_string()),
            app_name: Some("Google Chrome".to_string()),
            window_title: Some(title.to_string()),
            window_id: None,
            browser_url: Some(url.to_string()),
            display_id: Some(1),
            metadata_redaction_reason: None,
            metadata_redaction_source_id: None,
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
            "mnema CLI",
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
                speaker: None,
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
            "mnema CLI",
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
                speaker: None,
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
            "mnema CLI",
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
                speaker: None,
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
            "mnema CLI",
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
                speaker: None,
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
            "mnema CLI",
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
                speaker: None,
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
            "mnema CLI",
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
                speaker: None,
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
            "mnema CLI",
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
                speaker: None,
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
        turns: Vec::new(),
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
            "mnema CLI",
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
            let opaque_id = encode_signed_opaque_id("audio", segment.id, Some(&grant.id), &secret);

            let response = broker_show_text(
                &config_dir,
                &infra,
                std::slice::from_ref(&grant),
                &opaque_id,
            )
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
        let mapped =
            map_search_response(response, 5, None, Some("grant-1"), secret, HashMap::new());
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
            "mnema CLI",
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

        // The synthetic All Retained row is in-memory only: it authorizes, and it
        // never reaches the permission file, so Ask AI can never render as a
        // permission row with a Block button that would have to lie.
        assert!(load_grants(&config_dir).unwrap().grants.is_empty());

        // And it writes NO audit event. Ask AI runs an agent loop, so one event
        // per tool call evicted every real CLI event from the 500-slot FIFO
        // within a couple of dozen conversations (ADR 0059).
        assert!(load_audit_events(&config_dir).unwrap().events.is_empty());
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
                    speaker: None,
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
        let access = BrokeredCaptureAccess::from_config_dir(temp_config_dir("ask-ai-open").clone());
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
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id = encode_signed_opaque_id("frame", second.frame.id, Some(&grant.id), &secret);

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
            "mnema CLI",
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id = encode_signed_opaque_id("frame", second.frame.id, Some(&grant.id), &secret);

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
            "mnema CLI",
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
fn active_opaque_authorization_rejects_blocked_client_replay() {
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
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id = encode_signed_opaque_id("frame", frame.id, Some(&grant.id), &secret);

        assert!(block_client(&config_dir, "mnema CLI").expect("client should block"));

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
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let _other_grant = create_grant(
            &config_dir,
            "Other agent",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("other grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id =
            encode_signed_opaque_id("frame", frame.id, Some(&original_grant.id), &secret);

        assert!(block_client(&config_dir, "Original agent").expect("client should block"));

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
            "mnema CLI",
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
                        speaker: None,
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
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
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

fn speaker_turn_saying(
    provider_cluster_id: &str,
    start_ms: u64,
    end_ms: u64,
    text: &str,
) -> speaker_analysis::SpeakerTurn {
    speaker_analysis::SpeakerTurn {
        transcript_text: Some(text.to_string()),
        ..speaker_turn(provider_cluster_id, start_ms, end_ms)
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
    let grant = create_grant(config_dir, "mnema CLI", BrokerGrantScope::LAST_DAY)
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

        let shape: Vec<_> = response
            .speakers
            .iter()
            .map(|speaker| {
                (
                    speaker.name.as_deref(),
                    speaker.attribution.as_str(),
                    speaker.handle.kind.as_str(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (Some("Ada"), "assigned", "person"),
                (None, "unknown", "voice"),
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
            "mnema CLI",
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
        output.clusters[0].suggestion = Some(speaker_analysis::SpeakerRecognitionSuggestion {
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

        assert_eq!(response.speakers.len(), 1);
        assert_eq!(response.speakers[0].name, None);
        assert_eq!(response.speakers[0].attribution, "unknown");
        assert_eq!(response.speakers[0].confidence, None);
        assert_eq!(response.speakers[0].handle.kind, "voice");
        let json = serde_json::to_string(&response).expect("response should serialize");
        assert!(
            !json.contains("Ada"),
            "a rejected recognition must never reach an agent: {json}"
        );
    });
}

/// `turns` is what lets an agent quote the right person: every turn points at a
/// `speakers[]` index, so identity is decided once by the collapse and never
/// re-derived from a name string. `text` stays whole beside it — the turns are an
/// overlay on the transcript, not a replacement for it.
#[test]
fn broker_show_text_attributes_turns_to_the_speakers_it_publishes() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-turns");
        let save_dir = temp_save_dir("speaker-turns");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let (segment_id, grant, opaque_id) = seed_brokered_audio_segment(
            &config_dir,
            &save_dir,
            &infra,
            "attributed-session",
            "morning all morning Ada",
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
                "attributed-session",
                segment_id,
                vec![
                    speaker_cluster("speaker_00", &[1.0, 0.0]),
                    speaker_cluster("speaker_01", &[0.0, 1.0]),
                ],
                vec![
                    speaker_turn_saying("speaker_00", 0, 1_000, "morning all"),
                    speaker_turn_saying("speaker_01", 2_000, 3_000, "morning Ada"),
                ],
            ),
        )
        .await;
        let clusters = infra
            .list_speaker_clusters_for_session("attributed-session")
            .await
            .expect("clusters should list");
        let ada_cluster = clusters
            .iter()
            .find(|cluster| cluster.provider_cluster_id.ends_with("speaker_00"))
            .expect("the first cluster should exist");
        infra
            .link_speaker_cluster_to_person(ada_cluster.id, ada.id, false)
            .await
            .expect("cluster should link");

        let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run")
            .expect("audio should be authorized");

        let attributed: Vec<_> = response
            .turns
            .iter()
            .map(|turn| {
                let speaker = response
                    .speakers
                    .get(
                        turn.speaker
                            .expect("show-text always attributes to a speaker"),
                    )
                    .expect("every turn index must resolve into speakers[]");
                (
                    speaker.name.as_deref(),
                    speaker.handle.kind.as_str(),
                    turn.start_ms,
                    turn.end_ms,
                    turn.text.as_str(),
                )
            })
            .collect();
        assert_eq!(
            attributed,
            vec![
                (Some("Ada"), "person", 0, 1_000, "morning all"),
                (None, "voice", 2_000, 3_000, "morning Ada"),
            ]
        );
        assert_eq!(
            response.text, "morning all morning Ada",
            "turns are an overlay: the full transcript must survive beside them"
        );
    });
}

/// Diarization yields nothing for plenty of audio the transcriber handled fine.
/// That segment must still carry its words, with BOTH keys off the wire — absent
/// `turns` means "could not attribute", and an agent that finds an empty array
/// where the transcript is full would read the recording as silence.
#[test]
fn broker_show_text_without_diarization_omits_turns_and_speakers_from_the_wire() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-turns-undiarized");
        let save_dir = temp_save_dir("speaker-turns-undiarized");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let (_segment_id, grant, opaque_id) = seed_brokered_audio_segment(
            &config_dir,
            &save_dir,
            &infra,
            "undiarized-session",
            "a transcript nobody was attributed in",
        )
        .await;

        let response = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run")
            .expect("audio should be authorized");

        assert_eq!(response.text, "a transcript nobody was attributed in");
        let json = serde_json::to_value(&response).expect("response should serialize");
        assert!(
            json.get("turns").is_none() && json.get("speakers").is_none(),
            "unattributed audio must not publish empty attribution: {json}"
        );
    });
}

/// A speaker handle addresses a PERSON, and a person is not captured content.
/// It is signed by the same broker, so the only thing keeping it out of
/// `show-text`/`open` is its separate kind space — if that ever collided, an
/// agent could hand a person id to `open` and land on frame number 7.
#[test]
fn a_speaker_handle_is_not_a_capture_reference_for_show_text_or_open() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-handle-not-capture");
        let save_dir = temp_save_dir("speaker-handle-not-capture");
        write_recording_settings(&config_dir, &save_dir);
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let (segment_id, grant, opaque_id) = seed_brokered_audio_segment(
            &config_dir,
            &save_dir,
            &infra,
            "handle-session",
            "one voice",
        )
        .await;
        complete_speaker_analysis(
            &infra,
            segment_id,
            speaker_analysis_output(
                "handle-session",
                segment_id,
                vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
                vec![speaker_turn("speaker_00", 0, 1_000)],
            ),
        )
        .await;
        let response = broker_show_text(&config_dir, &infra, &[grant.clone()], &opaque_id)
            .await
            .expect("show text should run")
            .expect("audio should be authorized");
        let handle = response
            .speakers
            .first()
            .expect("the segment has a voice")
            .handle
            .id
            .clone();
        assert!(
            opaque_capture_reference(&handle).is_none(),
            "a speaker handle must never decode as a capture reference"
        );

        let shown = broker_show_text(&config_dir, &infra, &[grant], &handle)
            .await
            .expect("show text should run");
        assert_eq!(shown, Err(invalid_opaque_id_error()));

        let opened = BrokeredCaptureAccess::from_config_dir(config_dir.clone())
            .execute(
                "mnema-cli",
                BrokeredCaptureRequest::OpenInMnema {
                    opaque_id: handle.clone(),
                },
            )
            .await
            .expect("open should run");
        assert_eq!(
            opened,
            BrokeredCaptureResponse::Error(invalid_opaque_id_error()),
            "a person must not be openable as a capture"
        );
    });
}

/// Seeds one diarized audio segment over an EXPLICIT window, so a discovery test
/// can place a voice inside or outside the grant's time scope. One session per
/// call keeps the segment's unique file path unique too.
async fn seed_diarized_segment(
    save_dir: &Path,
    infra: &AppInfra,
    session_id: &str,
    started_at_ms: u64,
    ended_at_ms: u64,
    clusters: Vec<speaker_analysis::SpeakerCluster>,
    turns: Vec<speaker_analysis::SpeakerTurn>,
) -> i64 {
    let segment = infra
        .upsert_audio_segment(&NewAudioSegment::new(
            AudioSegmentSourceKind::Microphone,
            session_id,
            1,
            save_dir
                .join(format!("{session_id}.m4a"))
                .display()
                .to_string(),
            format_unix_ms(started_at_ms),
            format_unix_ms(ended_at_ms),
        ))
        .await
        .expect("segment should insert");
    complete_speaker_analysis(
        infra,
        segment.id,
        speaker_analysis_output(session_id, segment.id, clusters, turns),
    )
    .await;
    segment.id
}

async fn cluster_id_for(infra: &AppInfra, session_id: &str, provider_cluster_id: &str) -> i64 {
    infra
        .list_speaker_clusters_for_session(session_id)
        .await
        .expect("clusters should list")
        .into_iter()
        .find(|cluster| cluster.provider_cluster_id.ends_with(provider_cluster_id))
        .unwrap_or_else(|| panic!("{provider_cluster_id} should exist in {session_id}"))
        .id
}

async fn assign_cluster(
    infra: &AppInfra,
    session_id: &str,
    provider_cluster_id: &str,
    person_id: i64,
) {
    let cluster_id = cluster_id_for(infra, session_id, provider_cluster_id).await;
    infra
        .link_speaker_cluster_to_person(cluster_id, person_id, false)
        .await
        .expect("cluster should link");
}

fn recognized_cluster(
    provider_cluster_id: &str,
    embedding: &[f32],
    person_id: i64,
    display_name: &str,
) -> speaker_analysis::SpeakerCluster {
    speaker_analysis::SpeakerCluster {
        suggestion: Some(speaker_analysis::SpeakerRecognitionSuggestion {
            person_id,
            display_name: display_name.to_string(),
            confidence: speaker_analysis::RecognitionConfidence::High,
            score: 0.9,
        }),
        ..speaker_cluster(provider_cluster_id, embedding)
    }
}

async fn discover_speakers(
    config_dir: &Path,
    infra: &AppInfra,
    grant: &BrokerGrant,
    name: Option<&str>,
    limit: Option<u32>,
) -> BrokerSpeakersResponse {
    broker_speakers(
        config_dir,
        infra,
        std::slice::from_ref(grant),
        BrokerSpeakersRequest {
            name: name.map(str::to_string),
            limit,
        },
    )
    .await
    .expect("speakers should run")
    .expect("the grant should authorize the roster")
}

fn roster_shape(response: &BrokerSpeakersResponse) -> Vec<(Option<&str>, &str, u64)> {
    response
        .speakers
        .iter()
        .map(|speaker| {
            (
                speaker.name.as_deref(),
                speaker.handle.kind.as_str(),
                speaker.speaking_ms,
            )
        })
        .collect()
}

/// Discovery is what makes a handle reachable at all, so it must rank by how long
/// each voice actually spoke — not by name, not by insertion — and it must publish
/// the SAME handle `show-text` publishes for that person. A parallel identity here
/// would send an agent filtering on a handle nothing else recognizes.
#[test]
fn broker_speakers_ranks_named_and_unnamed_voices_by_speaking_time() {
    run_async_test(async {
        let config_dir = temp_config_dir("speakers-roster");
        let save_dir = temp_save_dir("speakers-roster");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let (segment_id, grant, opaque_id) = seed_brokered_audio_segment(
            &config_dir,
            &save_dir,
            &infra,
            "roster-session",
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
                "roster-session",
                segment_id,
                vec![
                    speaker_cluster("speaker_00", &[1.0, 0.0]),
                    speaker_cluster("speaker_01", &[0.0, 1.0]),
                ],
                vec![
                    speaker_turn("speaker_00", 0, 2_000),
                    speaker_turn("speaker_01", 3_000, 12_000),
                ],
            ),
        )
        .await;
        assign_cluster(&infra, "roster-session", "speaker_00", ada.id).await;

        let roster = discover_speakers(&config_dir, &infra, &grant, None, None).await;

        assert_eq!(
            roster_shape(&roster),
            vec![(None, "voice", 9_000), (Some("Ada"), "person", 2_000)],
            "the longer-speaking unnamed voice outranks the named person"
        );
        assert!(!roster.truncated);

        let shown = broker_show_text(&config_dir, &infra, &[grant], &opaque_id)
            .await
            .expect("show text should run")
            .expect("audio should be authorized");
        let ada_in_show_text = shown
            .speakers
            .iter()
            .find(|speaker| speaker.name.as_deref() == Some("Ada"))
            .expect("show-text publishes Ada");
        assert_eq!(
            roster.speakers[1].handle, ada_in_show_text.handle,
            "discovery must hand back the handle show-text already uses"
        );
    });
}

/// A ranked page that quietly dropped the rest reads to an agent as "that is
/// everyone". The flag is the only thing that says otherwise — and it must stay
/// clear when the cap did not bite, or it means nothing.
#[test]
fn broker_speakers_flags_truncation_only_when_the_cap_bites() {
    run_async_test(async {
        let config_dir = temp_config_dir("speakers-truncation");
        let save_dir = temp_save_dir("speakers-truncation");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let now = now_unix_ms();
        seed_diarized_segment(
            &save_dir,
            &infra,
            "truncation-session",
            now.saturating_sub(60 * 60 * 1000),
            now,
            vec![
                speaker_cluster("speaker_00", &[1.0, 0.0]),
                speaker_cluster("speaker_01", &[0.0, 1.0]),
            ],
            vec![
                speaker_turn("speaker_00", 0, 9_000),
                speaker_turn("speaker_01", 10_000, 11_000),
            ],
        )
        .await;

        let capped = discover_speakers(&config_dir, &infra, &grant, None, Some(1)).await;
        assert_eq!(capped.speakers.len(), 1);
        assert_eq!(capped.limit, 1);
        assert_eq!(capped.speakers[0].speaking_ms, 9_000);
        assert!(capped.truncated, "a dropped voice must be admitted to");

        let whole = discover_speakers(&config_dir, &infra, &grant, None, Some(5)).await;
        assert_eq!(whole.speakers.len(), 2);
        assert!(
            !whole.truncated,
            "the flag must stay clear when everyone fit: {whole:?}"
        );
    });
}

/// The "Skywalker" case: a person who barely spoke ranks below the cap and is
/// unreachable from the ranked page alone. The name fragment is the only way to
/// a handle for them — case-insensitively, on part of the name.
#[test]
fn broker_speakers_name_fragment_reaches_a_person_below_the_cap() {
    run_async_test(async {
        let config_dir = temp_config_dir("speakers-fragment");
        let save_dir = temp_save_dir("speakers-fragment");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let yoda = infra
            .create_person_profile("Yoda", None)
            .await
            .expect("person profile should insert");
        let anakin = infra
            .create_person_profile("Anakin Skywalker", None)
            .await
            .expect("person profile should insert");
        let now = now_unix_ms();
        seed_diarized_segment(
            &save_dir,
            &infra,
            "fragment-session",
            now.saturating_sub(60 * 60 * 1000),
            now,
            vec![
                speaker_cluster("speaker_00", &[1.0, 0.0]),
                speaker_cluster("speaker_01", &[0.0, 1.0]),
            ],
            vec![
                speaker_turn("speaker_00", 0, 20_000),
                speaker_turn("speaker_01", 21_000, 22_000),
            ],
        )
        .await;
        assign_cluster(&infra, "fragment-session", "speaker_00", yoda.id).await;
        assign_cluster(&infra, "fragment-session", "speaker_01", anakin.id).await;

        let ranked = discover_speakers(&config_dir, &infra, &grant, None, Some(1)).await;
        assert_eq!(
            roster_shape(&ranked),
            vec![(Some("Yoda"), "person", 20_000)]
        );

        let found =
            discover_speakers(&config_dir, &infra, &grant, Some("skywalker"), Some(1)).await;

        assert_eq!(
            roster_shape(&found),
            vec![(Some("Anakin Skywalker"), "person", 1_000)],
            "a half-remembered name must reach a person the ranking hid"
        );
        assert!(!found.truncated);
    });
}

/// `person_profiles.display_name` is plain TEXT, so two people really can share
/// one. Both must come back as separate handles: an agent that saw one row would
/// filter on one of them and silently answer for the wrong human.
#[test]
fn broker_speakers_gives_two_people_sharing_a_name_two_handles() {
    run_async_test(async {
        let config_dir = temp_config_dir("speakers-duplicate-name");
        let save_dir = temp_save_dir("speakers-duplicate-name");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let first = infra
            .create_person_profile("Ada Lovelace", None)
            .await
            .expect("person profile should insert");
        let second = infra
            .create_person_profile("Ada Lovelace", None)
            .await
            .expect("a second person may share the name");
        assert_ne!(first.id, second.id);
        let now = now_unix_ms();
        seed_diarized_segment(
            &save_dir,
            &infra,
            "duplicate-name-session",
            now.saturating_sub(60 * 60 * 1000),
            now,
            vec![
                speaker_cluster("speaker_00", &[1.0, 0.0]),
                speaker_cluster("speaker_01", &[0.0, 1.0]),
            ],
            vec![
                speaker_turn("speaker_00", 0, 5_000),
                speaker_turn("speaker_01", 6_000, 9_000),
            ],
        )
        .await;
        assign_cluster(&infra, "duplicate-name-session", "speaker_00", first.id).await;
        assign_cluster(&infra, "duplicate-name-session", "speaker_01", second.id).await;

        let found = discover_speakers(&config_dir, &infra, &grant, Some("lovelace"), None).await;

        assert_eq!(
            roster_shape(&found),
            vec![
                (Some("Ada Lovelace"), "person", 5_000),
                (Some("Ada Lovelace"), "person", 3_000),
            ]
        );
        assert_ne!(
            found.speakers[0].handle.id, found.speakers[1].handle.id,
            "one name, two people, two handles"
        );
    });
}

/// `person_profiles` is global; the grant is not. A roster that read the profile
/// table would name people the caller was never granted the audio for — the whole
/// reason this command is scoped rather than a people list.
#[test]
fn broker_speakers_never_names_a_person_heard_outside_the_grant_scope() {
    run_async_test(async {
        let config_dir = temp_config_dir("speakers-scope");
        let save_dir = temp_save_dir("speakers-scope");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let ada = infra
            .create_person_profile("Ada", None)
            .await
            .expect("person profile should insert");
        let bo = infra
            .create_person_profile("Bo", None)
            .await
            .expect("person profile should insert");
        let now = now_unix_ms();
        let day_ms = 24 * 60 * 60 * 1000;
        seed_diarized_segment(
            &save_dir,
            &infra,
            "in-scope-session",
            now.saturating_sub(60 * 60 * 1000),
            now,
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        seed_diarized_segment(
            &save_dir,
            &infra,
            "out-of-scope-session",
            now.saturating_sub(3 * day_ms),
            now.saturating_sub(3 * day_ms - 60 * 60 * 1000),
            vec![speaker_cluster("speaker_00", &[0.0, 1.0])],
            vec![speaker_turn("speaker_00", 0, 60_000)],
        )
        .await;
        assign_cluster(&infra, "in-scope-session", "speaker_00", ada.id).await;
        assign_cluster(&infra, "out-of-scope-session", "speaker_00", bo.id).await;

        let roster = discover_speakers(&config_dir, &infra, &grant, None, None).await;

        assert_eq!(
            roster_shape(&roster),
            vec![(Some("Ada"), "person", 1_000)],
            "the louder voice from three days ago is outside a one-day grant"
        );
        let by_name = discover_speakers(&config_dir, &infra, &grant, Some("bo"), None).await;
        assert!(
            by_name.speakers.is_empty(),
            "a name fragment must not reach past the grant either: {by_name:?}"
        );
    });
}

/// Both counts hang off ONE handle, because a person confirmed on one cluster and
/// merely voice-matched on another is one person. The split is how an agent weighs
/// how much of the answer is guesswork before it filters — an unnamed voice has
/// nothing confirmed and nothing recognized, and must say so.
#[test]
fn broker_speakers_reports_the_assigned_and_recognized_split_per_handle() {
    run_async_test(async {
        let config_dir = temp_config_dir("speakers-split");
        let save_dir = temp_save_dir("speakers-split");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let ada = infra
            .create_person_profile("Ada", None)
            .await
            .expect("person profile should insert");
        let now = now_unix_ms();
        seed_diarized_segment(
            &save_dir,
            &infra,
            "split-session",
            now.saturating_sub(60 * 60 * 1000),
            now,
            vec![
                // Deliberately far apart: near-identical embeddings resolve onto
                // ONE stable cluster row, which would collapse the split before
                // the broker ever sees it.
                speaker_cluster("speaker_00", &[1.0, 0.0]),
                recognized_cluster("speaker_01", &[0.0, 1.0], ada.id, "Ada"),
                speaker_cluster("speaker_02", &[-1.0, 0.0]),
            ],
            vec![
                speaker_turn("speaker_00", 0, 1_000),
                speaker_turn("speaker_00", 2_000, 3_000),
                speaker_turn("speaker_01", 4_000, 5_000),
                speaker_turn("speaker_02", 6_000, 7_000),
            ],
        )
        .await;
        assign_cluster(&infra, "split-session", "speaker_00", ada.id).await;

        let roster = discover_speakers(&config_dir, &infra, &grant, None, None).await;

        let split: Vec<_> = roster
            .speakers
            .iter()
            .map(|speaker| {
                (
                    speaker.name.as_deref(),
                    speaker.handle.kind.as_str(),
                    speaker.assigned_turns,
                    speaker.recognized_turns,
                )
            })
            .collect();
        assert_eq!(
            split,
            vec![(Some("Ada"), "person", 2, 1), (None, "voice", 0, 0),],
            "confirmed and guessed turns collapse onto one person handle"
        );
        assert_eq!(roster.speakers[0].speaking_ms, 3_000);
    });
}

/// `assignedTurns` is published to agents as "Turns the USER assigned to this
/// person. Confirmed identity." Owner-only auto-linking writes `person_id` with
/// nobody in the loop, so counting an auto-linked turn as assigned hands the
/// agent a confirmation the user was never asked for — the guesswork split it
/// weighs before filtering silently reads as settled.
#[test]
fn broker_speakers_counts_an_auto_linked_owner_turn_as_recognized() {
    run_async_test(async {
        let config_dir = temp_config_dir("speakers-auto-link");
        let save_dir = temp_save_dir("speakers-auto-link");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let owner = infra
            .upsert_account_owner_voiceprint(
                "You",
                "mock_speaker",
                "voice-model",
                &speaker_embedding_bytes(&[1.0, 0.0]),
            )
            .await
            .expect("owner voiceprint should store");
        let now = now_unix_ms();
        let segment = infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "auto-link-session",
                1,
                save_dir.join("auto-link-session.m4a").display().to_string(),
                format_unix_ms(now.saturating_sub(60 * 60 * 1000)),
                format_unix_ms(now),
            ))
            .await
            .expect("segment should insert");
        // The real auto-link path: a job frozen with "label my voice
        // automatically" on, completing with a High owner suggestion.
        let payload = crate::SpeakerAnalysisJobPayload {
            provider: "mock_speaker".to_string(),
            model_id: Some("voice-model".to_string()),
            recognize_people: true,
            auto_label_owner: true,
            options: serde_json::Map::new(),
        };
        let job = infra
            .enqueue_processing_job(
                &ProcessingJobDraft::for_audio_segment_speaker_analysis(segment.id)
                    .with_payload_json(
                        serde_json::to_string(&payload).expect("payload should encode"),
                    ),
            )
            .await
            .expect("speaker job should enqueue");
        infra
            .claim_queued_processing_job(job.id)
            .await
            .expect("speaker job should claim")
            .expect("claimed speaker job should exist");
        infra
            .complete_processing_job(
                job.id,
                &ProcessingResultDraft::new().with_structured_payload_json(
                    serde_json::to_string(&speaker_analysis_output(
                        "auto-link-session",
                        segment.id,
                        vec![recognized_cluster(
                            "speaker_00",
                            &[1.0, 0.0],
                            owner.id,
                            "You",
                        )],
                        vec![speaker_turn("speaker_00", 0, 1_000)],
                    ))
                    .expect("output should encode"),
                ),
            )
            .await
            .expect("speaker output should complete");

        let cluster = infra
            .list_speaker_clusters_for_session("auto-link-session")
            .await
            .expect("clusters should list")
            .into_iter()
            .next()
            .expect("cluster should exist");
        assert_eq!(cluster.person_id, Some(owner.id));
        assert!(
            cluster.person_link_auto,
            "the auto-linker must have decided this link for the test to mean anything"
        );

        let roster = discover_speakers(&config_dir, &infra, &grant, None, None).await;

        let split: Vec<_> = roster
            .speakers
            .iter()
            .map(|speaker| {
                (
                    speaker.name.as_deref(),
                    speaker.assigned_turns,
                    speaker.recognized_turns,
                )
            })
            .collect();
        assert_eq!(
            split,
            vec![(Some("You"), 0, 1)],
            "nobody confirmed this link, so no turn of it is assigned"
        );
    });
}

/// The dispatch, audit-name, and result-count arms in one pass — and the audit
/// row must show only THAT a speaker lookup ran. `record_audit_event` stores no
/// request parameters on purpose: the trail says a lookup happened, never who it
/// named.
#[test]
fn broker_speakers_is_audited_without_recording_who_it_named() {
    run_async_test(async {
        let config_dir = temp_config_dir("speakers-audit");
        let save_dir = temp_save_dir("speakers-audit");
        write_recording_settings(&config_dir, &save_dir);
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let ada = infra
            .create_person_profile("Ada", None)
            .await
            .expect("person profile should insert");
        let now = now_unix_ms();
        seed_diarized_segment(
            &save_dir,
            &infra,
            "audit-session",
            now.saturating_sub(60 * 60 * 1000),
            now,
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        assign_cluster(&infra, "audit-session", "speaker_00", ada.id).await;

        let response = BrokeredCaptureAccess::from_config_dir(config_dir.clone())
            .execute(
                "mnema-cli",
                BrokeredCaptureRequest::Speakers(BrokerSpeakersRequest {
                    name: Some("ada".to_string()),
                    limit: None,
                }),
            )
            .await
            .expect("speakers should run");

        let BrokeredCaptureResponse::Speakers(roster) = &response else {
            panic!("expected a Speakers response, got {response:?}");
        };
        assert_eq!(roster_shape(roster), vec![(Some("Ada"), "person", 1_000)]);

        let audit = load_audit_events(&config_dir).expect("audit should load");
        assert_eq!(audit.events.len(), 1);
        assert_eq!(audit.events[0].command_type, "speakers");
        assert_eq!(audit.events[0].result_count, 1);
        let json = serde_json::to_string(&audit).expect("audit should serialize");
        assert!(
            !json.to_lowercase().contains("ada"),
            "the audit trail must never record who a speaker lookup named: {json}"
        );
    });
}

/// One audio segment that is BOTH searchable and diarized: the completed
/// transcription projects the `search_documents` row `search` matches on, and the
/// speaker analysis writes the turns a speaker filter joins to. Either half alone
/// makes a filter test pass for the wrong reason.
async fn seed_searchable_diarized_segment(
    save_dir: &Path,
    infra: &AppInfra,
    session_id: &str,
    started_at: &str,
    ended_at: &str,
    transcript: &str,
    clusters: Vec<speaker_analysis::SpeakerCluster>,
    turns: Vec<speaker_analysis::SpeakerTurn>,
) -> i64 {
    let segment = infra
        .upsert_audio_segment(&NewAudioSegment::new(
            AudioSegmentSourceKind::Microphone,
            session_id,
            1,
            save_dir
                .join(format!("{session_id}.m4a"))
                .display()
                .to_string(),
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
    complete_speaker_analysis(
        infra,
        segment.id,
        speaker_analysis_output(session_id, segment.id, clusters, turns),
    )
    .await;
    segment.id
}

async fn search_with_speaker(
    config_dir: &Path,
    infra: &AppInfra,
    grants: &[BrokerGrant],
    query: &str,
    speaker: Option<&str>,
) -> Result<std::result::Result<BrokerSearchResponse, BrokerErrorResponse>> {
    broker_search(
        config_dir,
        infra,
        grants,
        BrokerSearchRequest {
            query: query.to_string(),
            from: None,
            to: None,
            limit: Some(20),
            app: None,
            window_title: None,
            url: None,
            url_regex: None,
            speaker: speaker.map(str::to_string),
            cursor: None,
        },
    )
    .await
}

fn matched_audio_segment_ids(response: &BrokerSearchResponse) -> Vec<i64> {
    let mut ids: Vec<i64> = response
        .results
        .iter()
        .filter_map(|result| opaque_capture_reference(&result.opaque_id))
        .filter_map(|reference| reference.audio_segment_id)
        .collect();
    ids.sort_unstable();
    ids
}

/// The handle an agent would actually filter on: the one discovery published for
/// this person, not one the test minted itself.
async fn discovered_person_handle(
    config_dir: &Path,
    infra: &AppInfra,
    grant: &BrokerGrant,
    name: &str,
) -> String {
    discover_speakers(config_dir, infra, grant, Some(name), None)
        .await
        .speakers
        .into_iter()
        .find(|speaker| speaker.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{name} should be discoverable"))
        .handle
        .id
}

/// The join that decides whose audio this is. Assignment wins and recognition
/// counts only where there is no assignment — `broker_collapse_speakers`'s
/// precedence verbatim, so the filter cannot contradict what `show-text` says
/// about the same recording. The third segment is the one a wrong join fails on:
/// the user assigned Bo over a guess of Priya, so it is BO's audio.
#[test]
fn broker_search_speaker_filter_matches_assigned_and_recognized_but_not_an_overridden_guess() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-filter-precedence");
        let save_dir = temp_save_dir("speaker-filter-precedence");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        let bo = infra
            .create_person_profile("Bo", None)
            .await
            .expect("person profile should insert");

        let assigned = seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "assigned-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "standup notes from the assigned recording",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        assign_cluster(&infra, "assigned-session", "speaker_00", priya.id).await;
        let recognized = seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "recognized-session",
            "2026-05-17T11:00:00Z",
            "2026-05-17T11:05:00Z",
            "standup notes from the recognized recording",
            vec![recognized_cluster(
                "speaker_00",
                &[0.0, 1.0],
                priya.id,
                "Priya",
            )],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        let overridden = seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "overridden-session",
            "2026-05-17T12:00:00Z",
            "2026-05-17T12:05:00Z",
            "standup notes from the overridden recording",
            vec![recognized_cluster(
                "speaker_00",
                &[-1.0, 0.0],
                priya.id,
                "Priya",
            )],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        // The user overruled the guess: this voice is Bo, whatever recognition said.
        assign_cluster(&infra, "overridden-session", "speaker_00", bo.id).await;

        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        let unfiltered =
            search_with_speaker(&config_dir, &infra, &[grant.clone()], "standup", None)
                .await
                .expect("search should run")
                .expect("search should be authorized");
        let mut all = vec![assigned, recognized, overridden];
        all.sort_unstable();
        assert_eq!(
            matched_audio_segment_ids(&unfiltered),
            all,
            "all three recordings must be reachable without the filter"
        );

        let filtered = search_with_speaker(&config_dir, &infra, &[grant], "standup", Some(&handle))
            .await
            .expect("search should run")
            .expect("search should be authorized");

        let mut expected = vec![assigned, recognized];
        expected.sort_unstable();
        assert_eq!(
            matched_audio_segment_ids(&filtered),
            expected,
            "a recognition the user overrode with someone else is not Priya's audio"
        );
    });
}

/// `recording_speaker_clusters.id` is `UNIQUE(session_id, provider,
/// provider_cluster_id)`, so a voice handle addresses ONE SESSION's voice. A
/// filter that reached another session would present two strangers as one human.
///
/// SESSION, not recording: both fixtures below are separate sessions, which is
/// the boundary this proves. Within one session the handle deliberately spans
/// every consecutive recording (`resolve_stable_speaker_cluster` reuses the row),
/// so this must not be read as proving a per-recording bound — there is none.
#[test]
fn broker_search_speaker_filter_by_voice_handle_stays_inside_its_own_session() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-filter-voice");
        let save_dir = temp_save_dir("speaker-filter-voice");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let first = seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "first-voice-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "handoff notes in the first recording",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        let second = seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "second-voice-session",
            "2026-05-17T11:00:00Z",
            "2026-05-17T11:05:00Z",
            "handoff notes in the second recording",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let secret = load_or_create_opaque_secret(&config_dir).expect("secret should load");
        let opaque_id = encode_signed_opaque_id("audio", first, Some(&grant.id), &secret);
        let handle = broker_show_text(
            &config_dir,
            &infra,
            std::slice::from_ref(&grant),
            &opaque_id,
        )
        .await
        .expect("show text should run")
        .expect("audio should be authorized")
        .speakers
        .first()
        .expect("the first recording has a voice")
        .handle
        .clone();
        assert_eq!(handle.kind, "voice");

        let filtered =
            search_with_speaker(&config_dir, &infra, &[grant], "handoff", Some(&handle.id))
                .await
                .expect("search should run")
                .expect("search should be authorized");

        assert_eq!(
            matched_audio_segment_ids(&filtered),
            vec![first],
            "an unnamed voice must not follow into another SESSION it was never heard in \
             (second session's recording: {second})"
        );
    });
}

/// A speaker filter narrows to audio, exactly as `screen_source` and
/// `audio_sources` already narrow each other: the voice is on the recording, and a
/// captured frame carries none. Without the narrowing an unrelated screenshot
/// would be served as something the person said.
#[test]
fn broker_search_speaker_filter_narrows_away_screen_results() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-filter-narrows");
        let save_dir = temp_save_dir("speaker-filter-narrows");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let frame = infra
            .insert_frame(&NewFrame::new(
                "screen-session",
                save_dir.join("retro.jpg").display().to_string(),
                "2026-05-17T10:02:00Z",
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
                &ProcessingResultDraft::new().with_result_text("retrospective agenda on screen"),
            )
            .await
            .expect("OCR job should complete");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        let spoken = seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "retro-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "retrospective agenda out loud",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        assign_cluster(&infra, "retro-session", "speaker_00", priya.id).await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        let unfiltered =
            search_with_speaker(&config_dir, &infra, &[grant.clone()], "retrospective", None)
                .await
                .expect("search should run")
                .expect("search should be authorized");
        assert!(
            unfiltered
                .results
                .iter()
                .any(|result| result.kind == "frame"),
            "the screen result must match without the filter: {unfiltered:?}"
        );

        let filtered = search_with_speaker(
            &config_dir,
            &infra,
            &[grant],
            "retrospective",
            Some(&handle),
        )
        .await
        .expect("search should run")
        .expect("search should be authorized");

        assert_eq!(matched_audio_segment_ids(&filtered), vec![spoken]);
        assert!(
            filtered
                .results
                .iter()
                .all(|result| result.kind.starts_with("audio")),
            "a speaker filter must leave only audio: {filtered:?}"
        );
    });
}

/// "What did Priya say in Zoom" sounds answerable and is not — the app lives on
/// frames, the voice lives on audio, and nothing joins them. An empty page here
/// would be reported to the user as "Priya said nothing in Zoom", so BOTH surfaces
/// refuse the combination outright instead of answering it.
#[test]
fn speaker_filter_combined_with_screen_filters_is_refused_on_search_and_timeline() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-filter-conflict");
        let save_dir = temp_save_dir("speaker-filter-conflict");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "conflict-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "planning the launch",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        assign_cluster(&infra, "conflict-session", "speaker_00", priya.id).await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        let error = broker_search(
            &config_dir,
            &infra,
            std::slice::from_ref(&grant),
            BrokerSearchRequest {
                query: "planning".to_string(),
                from: None,
                to: None,
                limit: Some(20),
                app: Some("Zoom".to_string()),
                window_title: None,
                url: None,
                url_regex: None,
                speaker: Some(handle.clone()),
                cursor: None,
            },
        )
        .await
        .expect_err("speaker + app must fail, never answer");
        assert!(
            matches!(&error, AppInfraError::InvalidSearchRequest(message)
                if message.contains("speaker cannot be combined")),
            "expected an explained refusal, got {error:?}"
        );

        let timeline_error = broker_timeline(
            &config_dir,
            &infra,
            std::slice::from_ref(&grant),
            BrokerTimelineRequest {
                from: "2026-05-17T00:00:00Z".to_string(),
                to: "2026-05-18T00:00:00Z".to_string(),
                limit: Some(5),
                app: None,
                window_title: None,
                url: Some("example.com".to_string()),
                url_regex: None,
                speaker: Some(handle.clone()),
            },
        )
        .await
        .expect_err("speaker + url must fail, never answer");
        assert!(
            matches!(&timeline_error, AppInfraError::InvalidSearchRequest(message)
                if message.contains("speaker cannot be combined")),
            "expected an explained refusal, got {timeline_error:?}"
        );

        // Not a blanket refusal, and not an empty answer dressed as one: the same
        // speaker filter without the screen filter returns the person's audio.
        let allowed = search_with_speaker(&config_dir, &infra, &[grant], "planning", Some(&handle))
            .await
            .expect("search should run")
            .expect("search should be authorized");
        assert_eq!(allowed.results.len(), 1);
    });
}

/// The same conflict, spelled the other way an agent can spell it: `app:`/`source:`
/// are QUERY OPERATORS that `search_capture` merges over the caller's refinements,
/// exactly as `before:`/`date:` do. The broker's conflict check only inspects the
/// request PARAMETERS, so "app:zoom planning" beside a speaker handle skips the
/// refusal, matches no frames (a speaker filter drops the frame pass) and no audio
/// (an app filter drops the audio pass), and comes back as a clean empty page —
/// which is precisely the "Priya said nothing in Zoom" the refusal exists to stop.
#[test]
fn broker_search_query_app_operator_cannot_smuggle_a_screen_filter_past_a_speaker_filter() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-app-operator-escape");
        let save_dir = temp_save_dir("speaker-app-operator-escape");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "operator-conflict-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "planning the launch",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn_saying(
                "speaker_00",
                0,
                1_000,
                "planning the launch",
            )],
        )
        .await;
        assign_cluster(&infra, "operator-conflict-session", "speaker_00", priya.id).await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        for query in ["app:zoom planning", "source:screen planning"] {
            let smuggled = search_with_speaker(
                &config_dir,
                &infra,
                std::slice::from_ref(&grant),
                query,
                Some(&handle),
            )
            .await;

            if let Ok(Ok(response)) = &smuggled {
                assert!(
                    !response.results.is_empty(),
                    "`{query}` beside a speaker filter answered with an empty page, \
                     which reads as \"she said nothing there\": {response:?}"
                );
            }
            assert!(
                matches!(&smuggled, Err(AppInfraError::InvalidSearchRequest(message))
                    if message.contains("speaker cannot be combined")),
                "the broker must refuse `{query}` beside a speaker filter: {smuggled:?}"
            );
        }

        // Still not a blanket refusal: the same speaker filter without a screen
        // operator answers.
        let allowed = search_with_speaker(&config_dir, &infra, &[grant], "planning", Some(&handle))
            .await
            .expect("search should run")
            .expect("search should be authorized");
        assert_eq!(allowed.results.len(), 1);
    });
}

/// "When was Priya talking yesterday" carries no query string, so the timeline is
/// the only surface that can answer it. Same narrowing as `search`: frames drop out
/// because a frame has no voice, and another voice's recording drops out too.
#[test]
fn broker_timeline_speaker_filter_returns_only_that_speakers_audio() {
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-speaker-filter");
        let save_dir = temp_save_dir("timeline-speaker-filter");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        seed_timeline_frame_with_browser_url(
            &infra,
            &save_dir,
            "timeline-speaker.jpg",
            "2026-05-17T10:30:00Z",
            None,
        )
        .await;
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        let hers = seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "priya-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "her recording",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        assign_cluster(&infra, "priya-session", "speaker_00", priya.id).await;
        seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "someone-else-session",
            "2026-05-17T11:00:00Z",
            "2026-05-17T11:05:00Z",
            "somebody else's recording",
            vec![speaker_cluster("speaker_00", &[0.0, 1.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        let timeline = |speaker: Option<String>| {
            let infra = &infra;
            let config_dir = config_dir.clone();
            let grant = grant.clone();
            async move {
                broker_timeline(
                    &config_dir,
                    infra,
                    &[grant],
                    BrokerTimelineRequest {
                        from: "2026-05-17T00:00:00Z".to_string(),
                        to: "2026-05-18T00:00:00Z".to_string(),
                        limit: Some(10),
                        app: None,
                        window_title: None,
                        url: None,
                        url_regex: None,
                        speaker,
                    },
                )
                .await
                .expect("timeline should run")
                .expect("timeline should be authorized")
            }
        };

        let unfiltered = timeline(None).await;
        assert_eq!(
            unfiltered
                .intervals
                .iter()
                .filter(|interval| interval.kind == "frame")
                .count(),
            1,
            "the screen interval must be there without the filter: {unfiltered:?}"
        );
        assert_eq!(
            unfiltered
                .intervals
                .iter()
                .filter(|interval| interval.kind.starts_with("audio"))
                .count(),
            2
        );

        let filtered = timeline(Some(handle)).await;

        assert_eq!(filtered.intervals.len(), 1, "{filtered:?}");
        assert_eq!(filtered.intervals[0].kind, "audio_microphone");
        assert_eq!(
            filtered.intervals[0]
                .opaque_id
                .as_deref()
                .and_then(opaque_capture_reference)
                .and_then(|reference| reference.audio_segment_id),
            Some(hers),
            "the one interval must be the recording she was heard in"
        );
    });
}

/// A handle the broker never signed — forged, mangled, or minted under a grant
/// that is gone — must be refused the way an invalid capture id already is. The
/// dangerous failures are the quiet ones: dropping the filter answers for
/// EVERYONE, and matching nothing answers "they said nothing".
#[test]
fn broker_speaker_filter_rejects_a_handle_this_broker_did_not_sign_or_no_longer_scopes() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-filter-forged");
        let save_dir = temp_save_dir("speaker-filter-forged");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "forged-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "briefing the team",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn("speaker_00", 0, 1_000)],
        )
        .await;
        assign_cluster(&infra, "forged-session", "speaker_00", priya.id).await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        let search = |speaker: String, grants: Vec<BrokerGrant>| {
            let infra = &infra;
            let config_dir = config_dir.clone();
            async move {
                search_with_speaker(&config_dir, infra, &grants, "briefing", Some(&speaker))
                    .await
                    .expect("search should run")
            }
        };

        assert_eq!(
            search("sp7.deadbeef".to_string(), vec![grant.clone()]).await,
            Err(invalid_speaker_handle_error()),
            "an unparseable handle must not be treated as no filter"
        );
        let mut tampered = handle.clone();
        let last = tampered.pop().expect("a handle ends in its signature");
        tampered.push(if last == '0' { '1' } else { '0' });
        assert_eq!(
            search(tampered, vec![grant.clone()]).await,
            Err(invalid_speaker_handle_error()),
            "a handle signed by nobody must be refused"
        );

        // Issued under a grant this caller no longer holds — the same gate a
        // capture reference passes through.
        let other_grant = create_grant(
            &config_dir,
            "Other agent",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        assert_eq!(
            search(handle.clone(), vec![other_grant]).await,
            Err(outside_scope_error()),
            "a handle outlives neither its grant nor its scope"
        );

        let timeline_rejection = broker_timeline(
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
                speaker: Some("not-a-handle".to_string()),
            },
        )
        .await
        .expect("timeline should run");
        assert_eq!(timeline_rejection, Err(invalid_speaker_handle_error()));
    });
}

/// The filter already joined `speaker_turns` to decide this recording is hers, so
/// the words it matched ride back on the result instead of costing a `show-text`
/// per hit. ONLY her turns: the same recording holds another voice, and shipping
/// that back would undo the narrowing the agent asked for. An UNFILTERED search
/// over the same rows adds no join and returns no turns at all — that is what
/// keeps this from being an N+1 on every ordinary search.
#[test]
fn broker_search_filtered_results_carry_only_the_matched_speakers_words() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-inline-turns");
        let save_dir = temp_save_dir("speaker-inline-turns");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        let segment = seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "inline-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "roadmap review, two voices",
            vec![
                speaker_cluster("speaker_00", &[1.0, 0.0]),
                speaker_cluster("speaker_01", &[0.0, 1.0]),
            ],
            vec![
                speaker_turn_saying("speaker_00", 0, 1_000, "ship the roadmap on Friday"),
                speaker_turn_saying("speaker_01", 1_000, 2_000, "I disagree entirely"),
                speaker_turn_saying("speaker_00", 2_000, 3_000, "noted, Friday it is"),
            ],
        )
        .await;
        assign_cluster(&infra, "inline-session", "speaker_00", priya.id).await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        let unfiltered =
            search_with_speaker(&config_dir, &infra, &[grant.clone()], "roadmap", None)
                .await
                .expect("search should run")
                .expect("search should be authorized");
        assert_eq!(matched_audio_segment_ids(&unfiltered), vec![segment]);
        assert!(
            unfiltered
                .results
                .iter()
                .all(|result| result.turns.is_empty()),
            "an unfiltered search asked about nobody, so it returns nobody's turns: {unfiltered:?}"
        );

        let filtered = search_with_speaker(&config_dir, &infra, &[grant], "roadmap", Some(&handle))
            .await
            .expect("search should run")
            .expect("search should be authorized");

        assert_eq!(matched_audio_segment_ids(&filtered), vec![segment]);
        let result = &filtered.results[0];
        assert_eq!(
            result
                .turns
                .iter()
                .map(|turn| (turn.start_ms, turn.end_ms, turn.text.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (0, 1_000, "ship the roadmap on Friday"),
                (2_000, 3_000, "noted, Friday it is"),
            ],
            "her words, in order, and nobody else's"
        );
        let json = serde_json::to_value(result).expect("result should serialize");
        assert!(
            json["turns"][0].get("speaker").is_none(),
            "one speaker per filtered result means no speakers[] to index into: {json}"
        );
    });
}

/// One recording can answer a search TWICE: two matched moments further apart
/// than the audio grouping gap are two anchors on the SAME audio segment. Both
/// results are that recording, so both must carry her words — an empty `turns`
/// on the second reads as "she said nothing here", the one thing this field must
/// never mean.
#[test]
fn broker_search_two_results_from_one_recording_both_carry_the_speakers_words() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-two-anchors");
        let save_dir = temp_save_dir("speaker-two-anchors");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        let segment = infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "two-anchor-session",
                1,
                save_dir
                    .join("two-anchor-session.m4a")
                    .display()
                    .to_string(),
                "2026-05-17T10:00:00Z",
                "2026-05-17T10:05:00Z",
            ))
            .await
            .expect("segment should insert");
        // Two transcript spans two minutes apart: past `AUDIO_GROUP_GAP_MS`, so
        // search groups them as two separate anchors on the one recording.
        let metadata = audio_transcription::TranscriptionMetadata {
            provider: "test".to_string(),
            model_id: None,
            language: "en".to_string(),
            segments: vec![
                audio_transcription::TranscriptionSegment {
                    start_ms: 0,
                    end_ms: 1_000,
                    text: "roadmap kickoff".to_string(),
                    confidence: None,
                },
                audio_transcription::TranscriptionSegment {
                    start_ms: 120_000,
                    end_ms: 121_000,
                    text: "roadmap wrapup".to_string(),
                    confidence: None,
                },
            ],
            words: Vec::new(),
            provenance: Default::default(),
        };
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
                &ProcessingResultDraft::new()
                    .with_result_text("roadmap kickoff and roadmap wrapup")
                    .with_structured_payload_json(
                        serde_json::to_string(&metadata).expect("metadata should serialize"),
                    ),
            )
            .await
            .expect("transcription should complete");
        complete_speaker_analysis(
            &infra,
            segment.id,
            speaker_analysis_output(
                "two-anchor-session",
                segment.id,
                vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
                vec![
                    speaker_turn_saying("speaker_00", 0, 1_000, "roadmap kickoff"),
                    speaker_turn_saying("speaker_00", 120_000, 121_000, "roadmap wrapup"),
                ],
            ),
        )
        .await;
        assign_cluster(&infra, "two-anchor-session", "speaker_00", priya.id).await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        let filtered = search_with_speaker(&config_dir, &infra, &[grant], "roadmap", Some(&handle))
            .await
            .expect("search should run")
            .expect("search should be authorized");

        assert_eq!(
            filtered.results.len(),
            2,
            "one recording, two matched moments: {filtered:?}"
        );
        assert!(
            filtered
                .results
                .iter()
                .all(|result| !result.turns.is_empty()),
            "both results are the SAME recording, so both carry her words: {filtered:?}"
        );
    });
}

/// "When was Priya talking yesterday" takes the same filter, so it gets the same
/// words: the timeline is the only surface that can answer it, and an interval
/// that says WHEN but not WHAT sends the agent back for a `show-text` the filter
/// already paid for.
#[test]
fn broker_timeline_filtered_intervals_carry_the_speakers_words() {
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-inline-turns");
        let save_dir = temp_save_dir("timeline-inline-turns");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "timeline-words-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "standup, two voices",
            vec![
                speaker_cluster("speaker_00", &[1.0, 0.0]),
                speaker_cluster("speaker_01", &[0.0, 1.0]),
            ],
            vec![
                speaker_turn_saying("speaker_00", 0, 1_000, "blocked on the migration"),
                speaker_turn_saying("speaker_01", 1_000, 2_000, "I can take that"),
            ],
        )
        .await;
        assign_cluster(&infra, "timeline-words-session", "speaker_00", priya.id).await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        let unfiltered = broker_timeline_over_may_17(&config_dir, &infra, &grant, None).await;
        assert!(
            unfiltered
                .intervals
                .iter()
                .all(|interval| interval.turns.is_empty()),
            "no speaker was named, so no words are attributed: {unfiltered:?}"
        );

        let filtered = broker_timeline_over_may_17(&config_dir, &infra, &grant, Some(handle)).await;

        assert_eq!(filtered.intervals.len(), 1, "{filtered:?}");
        assert_eq!(
            filtered.intervals[0]
                .turns
                .iter()
                .map(|turn| turn.text.as_str())
                .collect::<Vec<_>>(),
            vec!["blocked on the migration"],
            "the interval carries her words and only hers"
        );
    });
}

/// A grant is a TIME BOX and the broker is the only thing holding it: `from`/`to`
/// are clamped to the grant, but the QUERY STRING carries its own date operators
/// (`before:`/`after:`/`date:`) which `search_capture` merges over the caller's
/// date range last-write-wins. The broker must not let an agent's own query
/// re-open the window its grant closed — and with the speaker filter the payload
/// is no longer a 12-token snippet but that person's VERBATIM turns, so a widened
/// window hands back the whole transcript of audio this grant never covered.
#[test]
fn broker_search_query_date_operator_cannot_widen_the_grant_window() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-date-operator-escape");
        let save_dir = temp_save_dir("speaker-date-operator-escape");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        let now = now_unix_ms();
        seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "in-scope-session",
            &format_unix_ms(now.saturating_sub(60 * 60 * 1000)),
            &format_unix_ms(now),
            "roadmap standup today",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn_saying(
                "speaker_00",
                0,
                1_000,
                "roadmap standup today",
            )],
        )
        .await;
        assign_cluster(&infra, "in-scope-session", "speaker_00", priya.id).await;
        // Years outside a one-day grant.
        let out_of_scope = seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "old-session",
            "2020-03-04T10:00:00Z",
            "2020-03-04T10:05:00Z",
            "roadmap acquisition price is forty million",
            vec![speaker_cluster("speaker_01", &[0.0, 1.0])],
            vec![speaker_turn_saying(
                "speaker_01",
                0,
                1_000,
                "roadmap acquisition price is forty million",
            )],
        )
        .await;
        assign_cluster(&infra, "old-session", "speaker_01", priya.id).await;

        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::RecentDays { days: 1 },
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        let escaped = search_with_speaker(
            &config_dir,
            &infra,
            &[grant],
            "before:2021-01-01 roadmap",
            Some(&handle),
        )
        .await;

        let leaked = match &escaped {
            Ok(Ok(response)) => serde_json::to_string(response).expect("response serializes"),
            _ => String::new(),
        };
        assert!(
            !leaked.contains("forty million"),
            "a query date operator must not hand a one-day grant the verbatim words \
             of a 2020 recording: {escaped:?}"
        );
        if let Ok(Ok(response)) = &escaped {
            assert!(
                !matched_audio_segment_ids(response).contains(&out_of_scope),
                "a query date operator must not widen the grant's time box: {response:?}"
            );
        }
        // Refused out loud, never answered as an in-scope page: a silent narrowing
        // would read to the agent as "she said nothing before 2021".
        assert!(
            matches!(&escaped, Err(AppInfraError::InvalidSearchRequest(_))),
            "the broker must refuse a query that carries its own date window: {escaped:?}"
        );

        // The same window, asked the way the broker publishes it, still works and
        // is still clamped to the grant.
        let clamped = broker_search(
            &config_dir,
            &infra,
            &[create_grant(
                &config_dir,
                "mnema CLI",
                BrokerGrantScope::RecentDays { days: 1 },
            )
            .expect("grant should create")],
            BrokerSearchRequest {
                query: "roadmap".to_string(),
                from: Some("2020-01-01T00:00:00Z".to_string()),
                to: Some("2021-01-01T00:00:00Z".to_string()),
                limit: Some(20),
                app: None,
                window_title: None,
                url: None,
                url_regex: None,
                speaker: None,
                cursor: None,
            },
        )
        .await;
        assert!(
            !format!("{clamped:?}").contains("forty million"),
            "`from`/`to` stay clamped to the grant: {clamped:?}"
        );
    });
}

/// The two ways a speaker filter goes blind, counted apart because the remedies
/// are different: an unnamed voice is a labeling job the user can do, a recording
/// with no speaker data at all is a detection failure they cannot. One number for
/// both would tell an agent something is missing and nothing about what to do —
/// so each must also move on its own.
#[test]
fn broker_speaker_filter_counts_unnamed_voices_and_missing_speaker_data_apart() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-coverage-counts");
        let save_dir = temp_save_dir("speaker-coverage-counts");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "hers-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "her recording",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn_saying("speaker_00", 0, 1_000, "all done")],
        )
        .await;
        assign_cluster(&infra, "hers-session", "speaker_00", priya.id).await;
        // A voice nobody has named. It COULD be her; nothing here can tell.
        seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "unnamed-session",
            "2026-05-17T11:00:00Z",
            "2026-05-17T11:05:00Z",
            "somebody talking",
            vec![speaker_cluster("speaker_00", &[0.0, 1.0])],
            vec![speaker_turn_saying("speaker_00", 0, 1_000, "who is this")],
        )
        .await;
        // Transcribed fine, diarized into nothing — invisible to any filter.
        seed_undiarized_audio_segment(
            &save_dir,
            &infra,
            "no-speaker-data-session",
            "2026-05-17T12:00:00Z",
            "2026-05-17T12:05:00Z",
        )
        .await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let handle = discovered_person_handle(&config_dir, &infra, &grant, "Priya").await;

        let unfiltered = broker_timeline_over_may_17(&config_dir, &infra, &grant, None).await;
        assert_eq!(
            unfiltered.speaker_coverage, None,
            "nothing was filtered, so there is no blind spot to report — absent, not zeroed"
        );

        let filtered =
            broker_timeline_over_may_17(&config_dir, &infra, &grant, Some(handle.clone())).await;

        assert_eq!(
            filtered.speaker_coverage,
            Some(BrokerSpeakerCoverage {
                recordings_with_unnamed_voices: 1,
                recordings_without_speaker_data: 1,
            }),
            "one of each, counted apart"
        );

        // A second recording speaker detection produced nothing for moves ONE
        // count. A count that tracked the other would be reporting a total.
        seed_undiarized_audio_segment(
            &save_dir,
            &infra,
            "second-no-speaker-data-session",
            "2026-05-17T13:00:00Z",
            "2026-05-17T13:05:00Z",
        )
        .await;

        let again = broker_timeline_over_may_17(&config_dir, &infra, &grant, Some(handle)).await;

        assert_eq!(
            again.speaker_coverage,
            Some(BrokerSpeakerCoverage {
                recordings_with_unnamed_voices: 1,
                recordings_without_speaker_data: 2,
            }),
            "the detection-failure count moved on its own"
        );
    });
}

/// The workflow this whole slice exists for, end to end: `speakers` to learn the
/// handle, ONE filtered `search` to read what she said. No `show-text` anywhere —
/// if the words did not ride back on the results, answering this would cost one
/// more request per recording.
#[test]
fn speakers_then_one_filtered_search_answers_what_a_person_said() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-two-call-workflow");
        let save_dir = temp_save_dir("speaker-two-call-workflow");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let priya = infra
            .create_person_profile("Priya", None)
            .await
            .expect("person profile should insert");
        for (session, started_at, ended_at, said) in [
            (
                "monday-session",
                "2026-05-17T10:00:00Z",
                "2026-05-17T10:05:00Z",
                "the launch slips to June",
            ),
            (
                "tuesday-session",
                "2026-05-17T14:00:00Z",
                "2026-05-17T14:05:00Z",
                "we should tell the launch list",
            ),
        ] {
            seed_searchable_diarized_segment(
                &save_dir,
                &infra,
                session,
                started_at,
                ended_at,
                "launch chatter",
                vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
                vec![speaker_turn_saying("speaker_00", 0, 1_000, said)],
            )
            .await;
            assign_cluster(&infra, session, "speaker_00", priya.id).await;
        }
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        // Call 1: who is there, and how do I address her.
        let roster = discover_speakers(&config_dir, &infra, &grant, Some("Priya"), None).await;
        let handle = roster
            .speakers
            .first()
            .expect("Priya should be discoverable")
            .handle
            .id
            .clone();

        // Call 2: what did she say. Nothing else.
        let filtered = search_with_speaker(&config_dir, &infra, &[grant], "launch", Some(&handle))
            .await
            .expect("search should run")
            .expect("search should be authorized");

        let mut said: Vec<&str> = filtered
            .results
            .iter()
            .flat_map(|result| result.turns.iter().map(|turn| turn.text.as_str()))
            .collect();
        said.sort_unstable();
        assert_eq!(
            said,
            vec!["the launch slips to June", "we should tell the launch list"],
            "both recordings answer with her words, with no show-text in between"
        );
        assert!(
            filtered.speaker_coverage.is_some(),
            "a filtered answer always says how much audio it could not check"
        );
    });
}

/// An audio segment the transcriber handled and speaker detection did not — the
/// shape the `recordingsWithoutSpeakerData` count exists for.
async fn seed_undiarized_audio_segment(
    save_dir: &Path,
    infra: &AppInfra,
    session_id: &str,
    started_at: &str,
    ended_at: &str,
) {
    infra
        .upsert_audio_segment(&NewAudioSegment::new(
            AudioSegmentSourceKind::Microphone,
            session_id,
            1,
            save_dir
                .join(format!("{session_id}.m4a"))
                .display()
                .to_string(),
            started_at,
            ended_at,
        ))
        .await
        .expect("segment should insert");
}

async fn broker_timeline_over_may_17(
    config_dir: &Path,
    infra: &AppInfra,
    grant: &BrokerGrant,
    speaker: Option<String>,
) -> BrokerTimelineResponse {
    broker_timeline(
        config_dir,
        infra,
        std::slice::from_ref(grant),
        BrokerTimelineRequest {
            from: "2026-05-17T00:00:00Z".to_string(),
            to: "2026-05-18T00:00:00Z".to_string(),
            limit: Some(10),
            app: None,
            window_title: None,
            url: None,
            url_regex: None,
            speaker,
        },
    )
    .await
    .expect("timeline should run")
    .expect("timeline should be authorized")
}

/// `speakerCoverage` is the audio the filter had NO WAY to check — that is the
/// only reading under which "a non-zero count means this answer may be
/// incomplete" is true. A recording the filter matched and RETURNED is checked
/// audio, so it can never be part of the blind spot.
///
/// A `voice` handle is where the two collide: the handle addresses an unnamed
/// voice, so every recording it reaches is by definition "a recording holding a
/// voice nobody has named". Counted, the response admits it may be incomplete
/// about the very recordings that ARE the answer — and it does so on every single
/// voice-filtered query, which trains an agent to discount the count entirely.
#[test]
fn speaker_coverage_never_counts_a_recording_the_filter_returned() {
    run_async_test(async {
        let config_dir = temp_config_dir("speaker-coverage-self-report");
        let save_dir = temp_save_dir("speaker-coverage-self-report");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        let heard = seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "heard-session",
            "2026-05-17T10:00:00Z",
            "2026-05-17T10:05:00Z",
            "the voice that was asked about",
            vec![speaker_cluster("speaker_00", &[1.0, 0.0])],
            vec![speaker_turn_saying(
                "speaker_00",
                0,
                9_000,
                "this is the one",
            )],
        )
        .await;
        // A DIFFERENT unnamed voice, in its own session: genuinely out of the
        // filter's reach, and the count must keep reporting it.
        seed_searchable_diarized_segment(
            &save_dir,
            &infra,
            "stranger-session",
            "2026-05-17T11:00:00Z",
            "2026-05-17T11:05:00Z",
            "somebody else entirely",
            vec![speaker_cluster("speaker_00", &[0.0, 1.0])],
            vec![speaker_turn_saying("speaker_00", 0, 1_000, "who is this")],
        )
        .await;
        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");
        let roster = discover_speakers(&config_dir, &infra, &grant, None, None).await;
        let loudest = roster
            .speakers
            .first()
            .expect("a voice should be discoverable");
        assert_eq!(loudest.handle.kind, "voice");
        let handle = loudest.handle.id.clone();

        let filtered = broker_timeline_over_may_17(&config_dir, &infra, &grant, Some(handle)).await;

        assert_eq!(
            filtered
                .intervals
                .iter()
                .map(|interval| interval
                    .opaque_id
                    .as_deref()
                    .and_then(opaque_capture_reference)
                    .and_then(|reference| reference.audio_segment_id))
                .collect::<Vec<_>>(),
            vec![Some(heard)],
            "the voice handle reaches its own recording: {filtered:?}"
        );
        assert_eq!(
            filtered.speaker_coverage,
            Some(BrokerSpeakerCoverage {
                // The stranger's recording only. The returned one was checked.
                recordings_with_unnamed_voices: 1,
                recordings_without_speaker_data: 0,
            }),
            "a recording this answer already published is not audio the filter \
             could not check: {filtered:?}"
        );
    });
}

#[test]
fn scoped_date_range_normalizes_offset_bounds_to_utc() {
    // Offset-carrying bounds must come back as the SAME INSTANTS expressed with
    // `Z`. Capture rows are stored RFC3339-with-`Z` and the audio-segment overlap
    // predicate compares those strings lexicographically, so a surviving
    // `+05:30` suffix would sort as that wall clock in UTC and drop every row on
    // the far side of the UTC date boundary. An All Retained permission is the
    // unbounded case: there is no scope start to clamp against.
    let grant = ask_ai_all_retained_grant(&BrokerClientIdentity::default_cli());
    let range = scoped_date_range(
        &grant,
        Some("2020-03-05T00:00:00+05:30".to_string()),
        Some("2020-03-05T23:59:59+05:30".to_string()),
    )
    .expect("bounds parse")
    .refinement
    .expect("both bounds were supplied");

    assert_eq!(range.start_at, "2020-03-04T18:30:00Z");
    assert_eq!(range.end_at, "2020-03-05T18:29:59Z");

    // Already-`Z` bounds are untouched, so existing callers keep byte-identical
    // strings.
    let utc = scoped_date_range(
        &grant,
        Some("2020-03-04T18:30:00Z".to_string()),
        Some("2020-03-05T18:29:59Z".to_string()),
    )
    .expect("bounds parse")
    .refinement
    .expect("both bounds were supplied");
    assert_eq!(utc.start_at, range.start_at);
    assert_eq!(utc.end_at, range.end_at);
}

#[test]
fn timeline_page_reports_the_slice_it_actually_covers() {
    let interval = |started_at: &str| BrokerTimelineInterval {
        kind: "frame".to_string(),
        started_at: started_at.to_string(),
        ended_at: Some(started_at.to_string()),
        opaque_id: None,
        context: None,
        turns: Vec::new(),
    };

    // A full page is the window's NEWEST end, so the covered span — not the
    // requested window — is what the caller may reason about.
    let full = BrokerTimelineResponse::page(
        vec![
            interval("2026-08-15T17:49:28Z"),
            interval("2026-08-15T17:30:17Z"),
        ],
        2,
        None,
    );
    assert!(full.truncated, "a page that filled the limit may have more");
    assert_eq!(full.covered_from.as_deref(), Some("2026-08-15T17:30:17Z"));
    assert_eq!(full.covered_to.as_deref(), Some("2026-08-15T17:49:28Z"));

    // Bounds are derived from the intervals, not from their order: the same page
    // reversed must report the same span.
    let reversed = BrokerTimelineResponse::page(
        vec![
            interval("2026-08-15T17:30:17Z"),
            interval("2026-08-15T17:49:28Z"),
        ],
        2,
        None,
    );
    assert_eq!(reversed.covered_from, full.covered_from);
    assert_eq!(reversed.covered_to, full.covered_to);

    let partial = BrokerTimelineResponse::page(vec![interval("2026-08-15T17:49:28Z")], 20, None);
    assert!(!partial.truncated, "a short page saw the whole window");

    let empty = BrokerTimelineResponse::page(Vec::new(), 20, None);
    assert!(!empty.truncated);
    assert_eq!(empty.covered_from, None);
    assert_eq!(empty.covered_to, None);

    // Matches the CLI's long-standing reading: asking for nothing can never be a
    // complete answer.
    assert!(BrokerTimelineResponse::page(Vec::new(), 0, None).truncated);
}

/// The regression this exists for: an audio segment recorded at 03:21 local time
/// in a `+05:30` zone is stored as the PREVIOUS UTC day (`21:51Z`). Asking for
/// "my local today" with `+05:30` bounds used to drop it, because the audio
/// overlap predicate compares the bound string lexicographically against
/// `Z`-stored rows and `"2026-05-16T21:51:00Z" >= "2026-05-17T00:00:00+05:30"`
/// is false — while the frame half of the SAME call, comparing with
/// `julianday()`, saw it. One tool, two answers.
#[test]
fn broker_timeline_offset_bounds_reach_audio_on_the_far_side_of_the_utc_date() {
    run_async_test(async {
        let config_dir = temp_config_dir("timeline-offset-bounds");
        let save_dir = temp_save_dir("timeline-offset-bounds");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        // 2026-05-17 03:21 IST — early on the user's local day, previous UTC day.
        infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                save_dir
                    .join("early-local-morning.m4a")
                    .display()
                    .to_string(),
                "2026-05-16T21:51:00Z",
                "2026-05-16T21:56:00Z",
            ))
            .await
            .expect("audio segment should insert");

        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_timeline(
            &config_dir,
            &infra,
            &[grant],
            BrokerTimelineRequest {
                // The user's local day, expressed the way a client that knows the
                // user's offset naturally expresses it.
                from: "2026-05-17T00:00:00+05:30".to_string(),
                to: "2026-05-17T23:59:59+05:30".to_string(),
                limit: Some(5),
                app: None,
                window_title: None,
                url: None,
                url_regex: None,
                speaker: None,
            },
        )
        .await
        .expect("timeline should run")
        .expect("timeline should be authorized");

        assert_eq!(
            response
                .intervals
                .iter()
                .map(|interval| interval.started_at.as_str())
                .collect::<Vec<_>>(),
            vec!["2026-05-16T21:51:00Z"],
            "an offset-form local-day window must reach audio stored on the previous UTC day"
        );
        assert!(
            !response.truncated,
            "one interval under a limit of five is the whole window"
        );
        assert_eq!(
            response.covered_from.as_deref(),
            Some("2026-05-16T21:51:00Z")
        );
    });
}

/// Seed helpers for the `activities` door. Frames are inserted for real so the
/// headline lookup's existence join has something to find.
async fn seed_activity_with_frame(
    infra: &AppInfra,
    save_dir: &Path,
    title: &str,
    summary: &str,
    started_at_ms: i64,
    frame_captured_at: &str,
) -> i64 {
    use crate::user_context::store::{NewActivity, NewActivityEvidence};

    let frame = infra
        .insert_frame(
            &NewFrame::new(
                "screen-session",
                save_dir
                    .join(format!("{title}-{started_at_ms}.jpg"))
                    .display()
                    .to_string(),
                frame_captured_at,
            )
            .with_metadata_snapshot(capture_metadata::FrameMetadataSnapshot {
                app_bundle_id: Some("com.stablyai.orca".to_string()),
                app_name: Some("Orca".to_string()),
                window_title: Some("Orca".to_string()),
                window_id: None,
                browser_url: None,
                display_id: Some(1),
                metadata_redaction_reason: None,
                metadata_redaction_source_id: None,
            }),
        )
        .await
        .expect("frame should insert");

    infra
        .user_context()
        .insert_activity_with_evidence(NewActivity {
            title: title.to_string(),
            summary: summary.to_string(),
            category: None,
            focus: None,
            started_at_ms,
            ended_at_ms: started_at_ms + 60_000,
            derivation_run_id: None,
            evidence: vec![NewActivityEvidence {
                subject_type: "frame".to_string(),
                subject_id: frame.id,
                captured_at_ms: Some(started_at_ms),
                is_headline: true,
            }],
        })
        .await
        .expect("seed activity");

    frame.id
}

async fn seed_covering_run(infra: &AppInfra, start_ms: i64, end_ms: i64, status: &str) {
    use crate::user_context::store::{DistillationGateDrops, NewDerivationRun};

    infra
        .user_context()
        .insert_derivation_run(NewDerivationRun {
            kind: "activity".to_string(),
            window_start_ms: Some(start_ms),
            window_end_ms: Some(end_ms),
            status: status.to_string(),
            activities_derived: 0,
            conclusions_derived: 0,
            input_tokens: 0,
            output_tokens: 0,
            provider: None,
            model: None,
            error: None,
            gate_drops: DistillationGateDrops::default(),
        })
        .await
        .expect("seed derivation run");
}

/// The whole reason this door exists: a day-scale window comes back WHOLE and in
/// order, not as the newest page of a relevance ranking. `timeline` answers the
/// same window with its most recent handful of per-frame rows; this walks it.
#[test]
fn broker_activities_walks_the_window_oldest_first_and_uncapped() {
    run_async_test(async {
        let config_dir = temp_config_dir("activities-walk");
        let save_dir = temp_save_dir("activities-walk");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        // Seeded newest-first so a pass-through of insertion order would fail.
        for (offset, title) in [
            (3, "Evening review"),
            (1, "Morning triage"),
            (2, "Midday build"),
        ] {
            seed_activity_with_frame(
                &infra,
                &save_dir,
                title,
                "did the thing",
                1_000_000 + offset * 60_000,
                "2026-05-17T10:00:00Z",
            )
            .await;
        }
        seed_covering_run(&infra, 0, 10_000_000, "completed").await;

        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_activities(
            &config_dir,
            &infra,
            &[grant],
            BrokerActivitiesRequest {
                from: "1970-01-01T00:00:00Z".to_string(),
                to: "1970-01-01T02:00:00Z".to_string(),
            },
        )
        .await
        .expect("activities should run")
        .expect("activities should be authorized");

        assert_eq!(
            response
                .activities
                .iter()
                .map(|activity| activity.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Morning triage", "Midday build", "Evening review"],
            "the window is walked chronologically, not ranked"
        );
        assert!(!response.truncated);
        assert!(
            response
                .activities
                .iter()
                .all(|activity| activity.opaque_id.is_some()),
            "every episode with a surviving evidence frame is citable"
        );
        // The source card renders the EVIDENCE FRAME, so it must carry that
        // frame's app. Without this the card falls back to "Unknown app" even
        // though the app was captured and stored all along.
        let context = response.activities[0]
            .context
            .as_ref()
            .expect("the cited frame's app context must reach the card");
        assert_eq!(context.app_name.as_deref(), Some("Orca"));
        assert_eq!(context.app_bundle_id.as_deref(), Some("com.stablyai.orca"));
    });
}

/// LOAD-BEARING, exactly as in `recall_context`: an Activity's title/summary is
/// persisted UNFILTERED, so this broker-side guardrail is the only thing between
/// a sensitive episode and a cloud engine. Unlike `recall_context` there is no
/// query to hide behind here — an unfiltered window would hand over everything.
#[test]
fn sensitive_activity_never_egresses_via_activities() {
    run_async_test(async {
        let config_dir = temp_config_dir("activities-sensitive");
        let save_dir = temp_save_dir("activities-sensitive");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        seed_activity_with_frame(
            &infra,
            &save_dir,
            "Therapy appointment",
            "attended a therapy appointment",
            1_000_000,
            "2026-05-17T10:00:00Z",
        )
        .await;
        seed_activity_with_frame(
            &infra,
            &save_dir,
            "Reviewed the parser",
            "worked through the parser rewrite",
            1_060_000,
            "2026-05-17T10:01:00Z",
        )
        .await;
        seed_covering_run(&infra, 0, 10_000_000, "completed").await;

        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_activities(
            &config_dir,
            &infra,
            &[grant],
            BrokerActivitiesRequest {
                from: "1970-01-01T00:00:00Z".to_string(),
                to: "1970-01-01T02:00:00Z".to_string(),
            },
        )
        .await
        .expect("activities should run")
        .expect("activities should be authorized");

        assert_eq!(
            response
                .activities
                .iter()
                .map(|activity| activity.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Reviewed the parser"],
            "the sensitive episode must not cross the broker boundary"
        );
        let serialized = serde_json::to_string(&response).expect("response serializes");
        assert!(
            !serialized.to_lowercase().contains("therapy"),
            "no trace of the sensitive episode may reach the wire: {serialized}"
        );
    });
}

/// The anti-lie field. An empty (or short) list over an underived stretch must be
/// distinguishable from a genuinely quiet one, or the model reports "you did
/// nothing" for time that simply has not been summarized yet — the same failure
/// as reading a truncated `timeline` page as a whole day.
#[test]
fn broker_activities_reports_only_the_derived_slice_of_the_window() {
    run_async_test(async {
        let config_dir = temp_config_dir("activities-coverage");
        let save_dir = temp_save_dir("activities-coverage");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        seed_activity_with_frame(
            &infra,
            &save_dir,
            "Morning triage",
            "cleared the queue",
            1_000_000,
            "2026-05-17T10:00:00Z",
        )
        .await;
        // Derivation covered 600_000..2_000_000 only: the request below starts
        // before that and ends well after it.
        seed_covering_run(&infra, 600_000, 2_000_000, "completed").await;
        // A FAILED run over the later stretch summarized nothing, so it must not
        // advance the watermark.
        seed_covering_run(&infra, 2_000_000, 5_000_000, "failed").await;

        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_activities(
            &config_dir,
            &infra,
            &[grant],
            BrokerActivitiesRequest {
                from: "1970-01-01T00:00:00Z".to_string(),
                to: "1970-01-01T01:23:20Z".to_string(), // 5_000_000 ms
            },
        )
        .await
        .expect("activities should run")
        .expect("activities should be authorized");

        // Clamped to the intersection on BOTH edges: the window's own start is
        // before coverage began, and its end is past the last covering run.
        assert_eq!(
            response.derived_from.as_deref(),
            Some("1970-01-01T00:10:00Z"),
            "coverage starts where derivation started, not where the window did"
        );
        assert_eq!(
            response.derived_until.as_deref(),
            Some("1970-01-01T00:33:20Z"),
            "a failed run summarized nothing and must not raise the watermark"
        );
        assert_eq!(response.activities.len(), 1);
    });
}

/// Activities outlive the captures that grounded them (ADR 0029), so the evidence
/// rows carry no FK and can point at deleted frames. A dangling id would be worse
/// than none: the model would cite something `show_text` cannot open.
#[test]
fn broker_activities_never_hands_out_an_id_for_an_aged_out_frame() {
    run_async_test(async {
        use crate::user_context::store::{NewActivity, NewActivityEvidence};

        let config_dir = temp_config_dir("activities-retention");
        let save_dir = temp_save_dir("activities-retention");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");

        // The headline frame is aged out; a later evidence frame survives.
        let surviving = infra
            .insert_frame(&NewFrame::new(
                "screen-session",
                save_dir.join("surviving.jpg").display().to_string(),
                "2026-05-17T10:05:00Z",
            ))
            .await
            .expect("frame should insert");

        infra
            .user_context()
            .insert_activity_with_evidence(NewActivity {
                title: "Long episode".to_string(),
                summary: "spanned a retention boundary".to_string(),
                category: None,
                focus: None,
                started_at_ms: 1_000_000,
                ended_at_ms: 1_060_000,
                derivation_run_id: None,
                evidence: vec![
                    // Headline, but the frame no longer exists.
                    NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 999_999,
                        captured_at_ms: Some(1_000_000),
                        is_headline: true,
                    },
                    NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: surviving.id,
                        captured_at_ms: Some(1_030_000),
                        is_headline: false,
                    },
                ],
            })
            .await
            .expect("seed activity");

        // A second episode whose ONLY evidence is gone: citable by nothing.
        infra
            .user_context()
            .insert_activity_with_evidence(NewActivity {
                title: "Fully aged out".to_string(),
                summary: "every grounding frame is gone".to_string(),
                category: None,
                focus: None,
                started_at_ms: 1_100_000,
                ended_at_ms: 1_160_000,
                derivation_run_id: None,
                evidence: vec![NewActivityEvidence {
                    subject_type: "frame".to_string(),
                    subject_id: 999_998,
                    captured_at_ms: Some(1_100_000),
                    is_headline: true,
                }],
            })
            .await
            .expect("seed activity");
        seed_covering_run(&infra, 0, 10_000_000, "completed").await;

        let grant = create_grant(
            &config_dir,
            "mnema CLI",
            BrokerGrantScope::AllRetainedHistory,
        )
        .expect("grant should create");

        let response = broker_activities(
            &config_dir,
            &infra,
            &[grant],
            BrokerActivitiesRequest {
                from: "1970-01-01T00:00:00Z".to_string(),
                to: "1970-01-01T02:00:00Z".to_string(),
            },
        )
        .await
        .expect("activities should run")
        .expect("activities should be authorized");

        assert_eq!(response.activities.len(), 2, "both episodes still reported");
        assert!(
            response.activities[0].opaque_id.is_some(),
            "a dead headline falls back to the surviving evidence frame"
        );
        assert_eq!(
            response.activities[1].opaque_id, None,
            "no surviving evidence means no id, never a dangling one"
        );
    });
}

// ---------------------------------------------------------------------------
// ADR 0059 — CLI Access is a standing per-tool permission with idle expiry.
// ---------------------------------------------------------------------------

/// The invariant the whole redesign rests on: one row per identity. The old file
/// was append-only, so every agent session that skipped the documented preamble
/// minted another row and Settings rendered a graveyard.
#[test]
fn approving_the_same_client_twice_replaces_the_row_instead_of_appending() {
    let config_dir = temp_config_dir("upsert-replaces");

    create_grant(&config_dir, "Claude Code", BrokerGrantScope::LAST_DAY)
        .expect("first approval should store");
    create_grant(&config_dir, "Claude Code", BrokerGrantScope::LAST_DAY)
        .expect("second approval should store");
    // A different tool is a different permission, not a second row for this one.
    create_grant(&config_dir, "Codex", BrokerGrantScope::LAST_DAY)
        .expect("other tool should store");

    let stored = stored_grants(&config_dir);
    assert_eq!(stored.len(), 2, "one row per identity: {stored:?}");
    assert_eq!(
        stored
            .iter()
            .filter(|grant| grant.normalized_label == "claude code")
            .count(),
        1
    );
}

/// The highest-risk property in the plan. Opaque result ids are HMAC-signed
/// against the issuing grant id, so widening a permission must mutate the row in
/// place — a fresh id would fail re-authorization for every id already handed to
/// a running agent, mid-task.
#[test]
fn widening_a_permission_keeps_the_row_id_so_issued_opaque_ids_survive() {
    let config_dir = temp_config_dir("upgrade-keeps-id");

    let original = create_grant(&config_dir, "Claude Code", BrokerGrantScope::LAST_DAY)
        .expect("first approval should store");
    let widened = upsert_grant_for_identity(
        &config_dir,
        BrokerClientIdentity::new("Claude Code", BrokerClientIdentitySource::Explicit).unwrap(),
        BrokerGrantScope::AllRetainedHistory,
    )
    .expect("widen should store");

    assert!(!widened.created, "a widen is an upgrade, not a new row");
    assert_eq!(widened.grant.id, original.id);
    assert_eq!(widened.grant.scope, BrokerGrantScope::AllRetainedHistory);
    assert_eq!(
        widened.grant.created_at_unix_ms, original.created_at_unix_ms,
        "the row is the same permission, not a replacement"
    );

    let stored = stored_grants(&config_dir);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, original.id);
}

/// Blocking and idling out are different user intents. Idle expiry is benign
/// disuse and re-prompts; a block is a standing rejection that must not quietly
/// evaporate into a fresh prompt after 30 unused days.
#[test]
fn a_blocked_permission_is_never_idle_expired() {
    let config_dir = temp_config_dir("blocked-never-idles");
    create_grant(&config_dir, "Claude Code", BrokerGrantScope::LAST_DAY).expect("approval stores");
    assert!(block_client(&config_dir, "Claude Code").expect("block should apply"));

    // Age the block far past the idle threshold.
    with_grants_lock(&config_dir, |grants| {
        grants.grants[0].last_used_at_unix_ms = now_unix_ms()
            .saturating_sub(BROKER_GRANT_IDLE_TTL_MS)
            .saturating_sub(60 * 60 * 1000);
        save_grants_locked(&config_dir, grants)
    })
    .expect("ageing should write");

    let stored = stored_grants(&config_dir);
    assert_eq!(
        stored.len(),
        1,
        "a blocked row survives the prune: {stored:?}"
    );
    assert!(stored[0].blocked);
    assert!(stored[0].blocked_at_unix_ms.is_some());
    assert!(!grant_is_active(&stored[0], now_unix_ms()));

    // Re-enabling restores access without a prompt, and restarts the idle clock.
    assert!(unblock_client(&config_dir, "Claude Code").expect("unblock should apply"));
    let stored = stored_grants(&config_dir);
    assert!(!stored[0].blocked);
    assert!(grant_is_active(&stored[0], now_unix_ms()));
}

/// The stamp is coarse ON PURPOSE: a brokered read must stay a read (ADR 0041),
/// not pay a flocked file rewrite per call. Written non-canonically so that any
/// rewrite at all is visible in the bytes even when no field value would change.
#[test]
fn touch_last_used_does_not_rewrite_the_file_inside_the_stamp_interval() {
    let config_dir = temp_config_dir("touch-coarse");
    let grant = create_grant(&config_dir, "Claude Code", BrokerGrantScope::LAST_DAY)
        .expect("approval stores");

    let path = config_dir.join(BROKER_GRANTS_FILE_NAME);
    let compact = serde_json::to_string(&BrokerGrantFile {
        schema_version: 1,
        grants: vec![grant.clone()],
    })
    .expect("grants serialize");
    fs::write(&path, &compact).expect("compact file writes");

    touch_last_used(&config_dir, "claude code").expect("touch should run");
    assert_eq!(
        fs::read_to_string(&path).expect("file reads"),
        compact,
        "a fresh stamp must not rewrite the permission file"
    );

    // An hour-plus stale value does get stamped.
    with_grants_lock(&config_dir, |grants| {
        grants.grants[0].last_used_at_unix_ms = now_unix_ms().saturating_sub(2 * 60 * 60 * 1000);
        save_grants_locked(&config_dir, grants)
    })
    .expect("ageing should write");
    touch_last_used(&config_dir, "claude code").expect("touch should run");
    let stamped = stored_grants(&config_dir);
    assert!(
        now_unix_ms().saturating_sub(stamped[0].last_used_at_unix_ms) < 60 * 1000,
        "a stale permission is stamped: {stamped:?}"
    );
    assert_eq!(stamped[0].id, grant.id, "stamping never re-mints the id");
}

/// Nothing dead sits in the access list: it is a control surface, not a log.
#[test]
fn loading_prunes_idle_expired_rows_but_keeps_blocked_ones() {
    let config_dir = temp_config_dir("prune-on-load");
    let stale = now_unix_ms()
        .saturating_sub(BROKER_GRANT_IDLE_TTL_MS)
        .saturating_sub(60 * 60 * 1000);
    let row = |label: &str, blocked: bool| BrokerGrant {
        id: label.to_string(),
        label: label.to_string(),
        normalized_label: normalize_client_label(label).unwrap(),
        identity_source: BrokerClientIdentitySource::Explicit,
        created_at_unix_ms: stale,
        last_used_at_unix_ms: stale,
        scope: BrokerGrantScope::LAST_DAY,
        blocked,
        blocked_at_unix_ms: blocked.then_some(stale),
    };
    let mut live = row("Live tool", false);
    live.last_used_at_unix_ms = now_unix_ms();
    fs::write(
        config_dir.join(BROKER_GRANTS_FILE_NAME),
        serde_json::to_string(&BrokerGrantFile {
            schema_version: 1,
            grants: vec![row("Idle tool", false), row("Blocked tool", true), live],
        })
        .expect("grants serialize"),
    )
    .expect("file writes");

    let labels: Vec<String> = stored_grants(&config_dir)
        .into_iter()
        .map(|grant| grant.label)
        .collect();
    assert_eq!(labels, vec!["Blocked tool", "Live tool"]);

    // And the prune reaches disk the next time anything opens the lock.
    with_grants_lock(&config_dir, |_| Ok(())).expect("lock should open");
    let raw = fs::read_to_string(config_dir.join(BROKER_GRANTS_FILE_NAME)).expect("file reads");
    assert!(
        !raw.contains("Idle tool"),
        "idle row is gone from disk: {raw}"
    );
    assert!(raw.contains("Blocked tool"));
}

/// The live correctness bug slice 2 exists for: an agent that asked for two weeks
/// with a `lastDay` permission got one day of results and reported there was
/// nothing there. A confidently incomplete answer is the worst failure mode a
/// recall product has.
#[test]
fn a_request_reaching_past_the_permission_returns_results_and_says_it_was_clamped() {
    run_async_test(async {
        let config_dir = temp_config_dir("clamp-marker");
        let save_dir = temp_save_dir("clamp-marker");
        let infra = AppInfra::initialize(&save_dir)
            .await
            .expect("infra should initialize");
        write_recording_settings(&config_dir, &save_dir);
        let now = now_unix_ms();
        let recent = format_unix_ms(now.saturating_sub(60 * 60 * 1000));
        infra
            .upsert_audio_segment(&NewAudioSegment::new(
                AudioSegmentSourceKind::Microphone,
                "mic-session",
                1,
                save_dir.join("audio.m4a").display().to_string(),
                recent.clone(),
                recent.clone(),
            ))
            .await
            .expect("segment should insert");

        let grant = create_grant(&config_dir, "mnema CLI", BrokerGrantScope::LAST_DAY)
            .expect("grant stores");
        let timeline = |from: String| {
            let grant = grant.clone();
            let config_dir = config_dir.clone();
            let infra = &infra;
            async move {
                broker_timeline(
                    &config_dir,
                    infra,
                    &[grant],
                    BrokerTimelineRequest {
                        from,
                        to: format_unix_ms(now),
                        limit: Some(50),
                        app: None,
                        window_title: None,
                        url: None,
                        url_regex: None,
                        speaker: None,
                    },
                )
                .await
                .expect("timeline should run")
                .expect("timeline should authorize")
            }
        };

        let clamped = timeline(format_unix_ms(now.saturating_sub(14 * 24 * 60 * 60 * 1000))).await;
        assert!(
            !clamped.intervals.is_empty(),
            "the in-scope slice still comes back"
        );
        assert!(clamped.scope_clamped);
        assert_eq!(clamped.required_scope.as_deref(), Some("allRetained"));

        // Eight days back needs a week-plus, which is still `allRetained`; two
        // days back is the `last7Days` band.
        let two_days = timeline(format_unix_ms(now.saturating_sub(2 * 24 * 60 * 60 * 1000))).await;
        assert!(two_days.scope_clamped);
        assert_eq!(two_days.required_scope.as_deref(), Some("last7Days"));

        // Inside the permission, nothing was narrowed and nothing is marked.
        let unclamped = timeline(format_unix_ms(now.saturating_sub(60 * 60 * 1000))).await;
        assert!(!unclamped.intervals.is_empty());
        assert!(!unclamped.scope_clamped);
        assert_eq!(unclamped.required_scope, None);

        // `search` carries the same marker, and this is the shape of the actual
        // bug: an EMPTY page for a window the caller was never allowed to see.
        // Without the marker that reads as "nothing happened in two weeks".
        let search = |from: String| {
            let grant = grant.clone();
            let config_dir = config_dir.clone();
            let infra = &infra;
            async move {
                broker_search(
                    &config_dir,
                    infra,
                    &[grant],
                    BrokerSearchRequest {
                        query: "roadmap".to_string(),
                        from: Some(from),
                        to: Some(format_unix_ms(now)),
                        limit: Some(20),
                        app: None,
                        window_title: None,
                        url: None,
                        url_regex: None,
                        speaker: None,
                        cursor: None,
                    },
                )
                .await
                .expect("search should run")
                .expect("search should authorize")
            }
        };
        let clamped = search(format_unix_ms(now.saturating_sub(14 * 24 * 60 * 60 * 1000))).await;
        assert!(clamped.results.is_empty());
        assert!(clamped.scope_clamped);
        assert_eq!(clamped.required_scope.as_deref(), Some("allRetained"));

        let unclamped = search(format_unix_ms(now.saturating_sub(60 * 60 * 1000))).await;
        assert!(!unclamped.scope_clamped);
        assert_eq!(unclamped.required_scope, None);
    });
}

#[test]
fn minimum_scope_for_start_maps_each_band() {
    let now = now_unix_ms();
    let ago = |ms: u64| now.saturating_sub(ms);
    assert_eq!(
        minimum_scope_for_start(ago(60 * 60 * 1000), now),
        BrokerGrantScope::LAST_DAY
    );
    assert_eq!(
        minimum_scope_for_start(ago(3 * 24 * 60 * 60 * 1000), now),
        BrokerGrantScope::LAST_7_DAYS
    );
    assert_eq!(
        minimum_scope_for_start(ago(30 * 24 * 60 * 60 * 1000), now),
        BrokerGrantScope::AllRetainedHistory
    );
    assert_eq!(BrokerGrantScope::LAST_DAY.wire_name(), "lastDay");
    assert_eq!(BrokerGrantScope::LAST_7_DAYS.wire_name(), "last7Days");
    assert_eq!(
        BrokerGrantScope::AllRetainedHistory.wire_name(),
        "allRetained"
    );
    assert_eq!(
        BrokerGrantScope::from_wire_name("last7Days"),
        Some(BrokerGrantScope::LAST_7_DAYS)
    );
    assert!(BrokerGrantScope::AllRetainedHistory.covers(&BrokerGrantScope::LAST_7_DAYS));
    assert!(!BrokerGrantScope::LAST_DAY.covers(&BrokerGrantScope::LAST_7_DAYS));
    assert!(BrokerGrantScope::LAST_7_DAYS.covers(&BrokerGrantScope::LAST_DAY));
}
