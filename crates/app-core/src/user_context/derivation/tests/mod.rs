use super::*;

/// The engine schema must not contain `$ref`/`$defs`: reference-less
/// structured-output backends reject `Error resolving schema reference`.
/// `#[schemars(inline)]` on the nested item types keeps both batch schemas
/// flat — guard that here so a future field/type change cannot silently
/// reintroduce a `$ref`.
#[test]
fn batch_schemas_are_reference_free() {
    // Walk the JSON structurally: a real reference is an object *key*
    // (`$ref`/`$defs`), not the literal text that appears inside a
    // `description` copied from these types' doc comments.
    fn has_reference(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                map.contains_key("$ref")
                    || map.contains_key("$defs")
                    || map.values().any(has_reference)
            }
            serde_json::Value::Array(items) => items.iter().any(has_reference),
            _ => false,
        }
    }

    for schema in [
        serde_json::to_value(schemars::schema_for!(DerivedActivityBatch)).unwrap(),
        serde_json::to_value(schemars::schema_for!(DistilledConclusionBatch)).unwrap(),
    ] {
        assert!(!has_reference(&schema), "schema has a reference: {schema}");
    }
}

#[test]
fn parses_frame_and_audio_refs() {
    assert_eq!(parse_ref("f12"), Some(("frame", 12)));
    assert_eq!(parse_ref(" a3 "), Some(("audio_segment", 3)));
    assert_eq!(parse_ref("x9"), None);
    assert_eq!(parse_ref("frame"), None);
}

#[test]
fn parses_known_categories_only() {
    for (raw, expected) in [
        ("creating", ActivityCategory::Creating),
        ("communication", ActivityCategory::Communication),
        ("meetings", ActivityCategory::Meetings),
        ("research", ActivityCategory::Research),
        ("learning", ActivityCategory::Learning),
        ("organizing", ActivityCategory::Organizing),
        ("personal", ActivityCategory::Personal),
        ("entertainment", ActivityCategory::Entertainment),
    ] {
        assert_eq!(
            parse_category(&Some(raw.to_string())),
            Some(expected),
            "{raw}"
        );
    }
    assert_eq!(parse_category(&Some("unknown".to_string())), None);
    assert_eq!(parse_category(&Some("  ".to_string())), None);
    assert_eq!(parse_category(&None), None);
}

#[test]
fn parse_category_aliases_legacy_spellings() {
    for (raw, expected) in [
        ("coding", ActivityCategory::Creating),
        ("Coding", ActivityCategory::Creating),
        ("design", ActivityCategory::Creating),
        ("testing", ActivityCategory::Creating),
        ("distractions", ActivityCategory::Entertainment),
    ] {
        assert_eq!(
            parse_category(&Some(raw.to_string())),
            Some(expected),
            "{raw}"
        );
    }
}

#[test]
fn parses_known_focus_levels_only() {
    assert_eq!(
        parse_focus(&Some("Deep".to_string())),
        Some(FocusLevel::Deep)
    );
    assert_eq!(
        parse_focus(&Some(" mixed ".to_string())),
        Some(FocusLevel::Mixed)
    );
    assert_eq!(
        parse_focus(&Some("distracted".to_string())),
        Some(FocusLevel::Distracted)
    );
    assert_eq!(parse_focus(&Some("unknown".to_string())), None);
    assert_eq!(parse_focus(&None), None);
}

fn correction(
    id: i64,
    title: &str,
    category: Option<ActivityCategory>,
    focus: Option<FocusLevel>,
) -> ActivityCorrection {
    ActivityCorrection {
        activity_id: id,
        title: title.to_string(),
        summary: format!("{title} summary"),
        corrected_category: category,
        corrected_focus: focus,
        corrected_at_ms: 1_000,
    }
}

#[test]
fn corrections_block_is_empty_without_corrections() {
    assert!(build_corrections_block(&[]).is_empty());
}

#[test]
fn corrections_block_lists_each_correction_with_labels() {
    let corrections = vec![
        correction(
            1,
            "Scrolled social media",
            Some(ActivityCategory::Entertainment),
            Some(FocusLevel::Distracted),
        ),
        // A correction that unset both labels renders as "unset".
        correction(2, "Misc", None, None),
    ];
    let block = build_corrections_block(&corrections);
    assert!(block.contains("USER CORRECTIONS"));
    assert!(block.contains("never re-assign the label they corrected away"));
    assert!(block.contains("category=entertainment focus=distracted"));
    assert!(block.contains("Scrolled social media"));
    assert!(block.contains("category=unset focus=unset"));
}

#[test]
fn corrections_block_respects_char_cap() {
    let long = "x".repeat(200);
    let corrections: Vec<ActivityCorrection> = (0..50)
        .map(|i| {
            correction(
                i,
                &long,
                Some(ActivityCategory::Creating),
                Some(FocusLevel::Deep),
            )
        })
        .collect();
    let block = build_corrections_block(&corrections);
    assert!(block.contains("USER CORRECTIONS"));
    assert!(
        block.chars().count() < CORRECTION_FEEDBACK_CHAR_CAP + 800,
        "block stays bounded by the char cap (+header/one-line slack)"
    );
}

#[test]
fn truncates_long_text_on_char_boundary() {
    let text = "a".repeat(ITEM_TEXT_CHAR_CAP + 50);
    let truncated = truncate_chars(&text, ITEM_TEXT_CHAR_CAP);
    // cap chars + the ellipsis.
    assert_eq!(truncated.chars().count(), ITEM_TEXT_CHAR_CAP + 1);
}

#[test]
fn supersedes_id_defaults_to_none_when_absent() {
    // ADR 0046: a draft with no supersedes_id (the common case) parses to None,
    // exactly like reinforces_id — the `#[serde(default)]` must hold.
    let json = r#"{"subject":"Rust","statement":"Rust is compiled","support_refs":[1,2]}"#;
    let parsed: DistilledConclusion = serde_json::from_str(json).expect("parse");
    assert_eq!(parsed.supersedes_id, None);
    assert_eq!(parsed.reinforces_id, None);
    // And it round-trips when present.
    let with = r#"{"subject":"Rust","statement":"x","support_refs":[1],"supersedes_id":7}"#;
    let parsed: DistilledConclusion = serde_json::from_str(with).expect("parse");
    assert_eq!(parsed.supersedes_id, Some(7));
}

#[test]
fn dismissed_block_marks_supersede_rows_as_already_replaced() {
    // ADR 0046 Slice 3H: user Dismiss rows read as "rejected", machine
    // supersede rows as "already replaced"; both stay in the block.
    let mut user = dismissal("Rust", "Rust is interpreted", "1,2", 2);
    user.source = "user".to_string();
    let mut sup = dismissal("Go", "Go has classes", "3,4", 2);
    sup.source = "supersede".to_string();
    let block = build_dismissed_conclusions_block(&[user, sup]);
    assert!(block.contains("(rejected) subject=Rust"));
    assert!(block.contains("(already replaced) subject=Go"));
}

#[test]
fn support_span_dates_by_supporting_activity_time_not_now() {
    let started: HashMap<i64, i64> = [(1, 3_000), (2, 1_000), (3, 2_000)].into();
    // Formed = earliest supporting activity, last supported = latest; the
    // distillation run time (`now`) never leaks in while any ref resolves.
    assert_eq!(
        support_span_ms(&[1, 2, 3], &started, 99_000),
        (1_000, 3_000)
    );
    assert_eq!(support_span_ms(&[3], &started, 99_000), (2_000, 2_000));
    assert_eq!(support_span_ms(&[], &started, 99_000), (99_000, 99_000));
}

#[test]
fn filter_known_refs_drops_unknown_and_dedups() {
    let valid: std::collections::HashSet<i64> = [1, 2, 3].into_iter().collect();
    // 9 is not in the set; the second `2` is a duplicate; order preserved.
    let filtered = filter_known_refs(&[2, 9, 1, 2, 3], &valid);
    assert_eq!(filtered, vec![2, 1, 3]);
    // No valid refs at all => empty (the caller skips ungrounded conclusions).
    assert!(filter_known_refs(&[9, 10], &valid).is_empty());
}

fn dismissal(subject: &str, statement: &str, fingerprint: &str, count: i64) -> DismissalState {
    DismissalState {
        subject: subject.to_string(),
        statement: statement.to_string(),
        evidence_fingerprint: fingerprint.to_string(),
        evidence_activity_count: count,
        dismissed_at_ms: 1_000,
        source: "user".to_string(),
    }
}

mod prompt_blocks;

// === ADR 0050: source-aware, name-blind audio derivation ================

/// Build a minimal audio [`CaptureWindowItem`] for the prompt-builder tests.
fn audio_item(
    id: i64,
    t: i64,
    text: &str,
    source: &str,
    turns: Vec<(i64, String)>,
) -> app_infra::CaptureWindowItem {
    app_infra::CaptureWindowItem {
        subject_type: "audio_segment".to_string(),
        subject_id: id,
        captured_at_ms: t,
        text: text.to_string(),
        app_label: None,
        url: None,
        source_kind: Some(source.to_string()),
        speaker_turns: turns,
    }
}

#[test]
fn speaker_turns_map_clusters_to_a_b_deterministically() {
    // Two distinct cluster ids → Speaker A / Speaker B in first-appearance
    // order; a repeat of the first cluster reuses "Speaker A".
    let rendered = render_speaker_turns(&[
        (42, "hello".to_string()),
        (7, "hi there".to_string()),
        (42, "how are you".to_string()),
    ]);
    assert_eq!(
        rendered,
        "Speaker A: hello\nSpeaker B: hi there\nSpeaker A: how are you\n"
    );
    // Empty / whitespace-only turns render nothing.
    assert!(render_speaker_turns(&[]).is_empty());
    assert!(render_speaker_turns(&[(1, "   ".to_string())]).is_empty());
}

#[test]
fn whitespace_turn_consumes_no_speaker_label() {
    // Invariant 3 (ADR 0050): a whitespace-only turn is skipped BEFORE the
    // A/B label index is computed, so it never burns a Speaker slot on a
    // LATER distinct cluster. Cluster 1 is whitespace → gets NO label; the
    // first REAL voice (cluster 2) is still Speaker A, not Speaker B.
    assert_eq!(
        render_speaker_turns(&[(1, "   ".to_string()), (2, "hi".to_string())]),
        "Speaker A: hi\n"
    );
    // Contrast: a REAL first turn does take Speaker A, pushing the next
    // distinct cluster to Speaker B — proving the label index tracks the
    // order of real (non-whitespace) turns.
    assert_eq!(
        render_speaker_turns(&[(1, "hey".to_string()), (2, "hi".to_string())]),
        "Speaker A: hey\nSpeaker B: hi\n"
    );
}

#[test]
fn source_tag_maps_only_known_audio_sources() {
    assert_eq!(source_tag("audio_segment", Some("microphone")), Some("you"));
    assert_eq!(
        source_tag("audio_segment", Some("system_audio")),
        Some("computer")
    );
    // Unknown / absent source, and any non-audio subject, get no tag.
    assert_eq!(
        (
            source_tag("audio_segment", None),
            source_tag("audio_segment", Some("bogus"))
        ),
        (None, None)
    );
    assert_eq!(source_tag("frame", Some("microphone")), None);
}

#[test]
fn build_prompt_tags_source_and_renders_anonymous_turns() {
    let window = CaptureWindow {
        start_ms: 1_000,
        end_ms: 2_000,
        items: vec![
            audio_item(
                1,
                1_100,
                "let me walk you through it",
                "microphone",
                vec![(10, "let me walk you through it".to_string())],
            ),
            audio_item(
                2,
                1_200,
                "sounds good thanks",
                "system_audio",
                vec![(20, "sounds good".to_string()), (21, "thanks".to_string())],
            ),
        ],
    };
    let prompt = build_prompt(&window);
    // The mic item is tagged `you`; the system-audio item `computer`.
    assert!(prompt.contains("[a1] t="));
    assert!(prompt.contains("source=you"));
    assert!(prompt.contains("[a2] t="));
    assert!(prompt.contains("source=computer"));
    // The two distinct voices in the system-audio segment render as A/B.
    assert!(prompt.contains("Speaker A: sounds good"));
    assert!(prompt.contains("Speaker B: thanks"));
    // The intro explains the attribution rule, and that computer audio can be media.
    assert!(
        prompt.contains("never render computer audio as words the user said")
            && prompt.contains("it is a conversation only")
    );
}

#[test]
fn build_prompt_separates_items_with_a_blank_line() {
    // Invariant 4: consecutive items stay separated by a blank line so the
    // model can tell one item's body from the next's. This guards the
    // trailing-newline layout: a future edit that drops a per-item newline
    // must not silently merge two items into one undelimited block.
    let window = CaptureWindow {
        start_ms: 0,
        end_ms: 10,
        items: vec![
            audio_item(1, 1, "alpha", "microphone", vec![]),
            audio_item(2, 2, "beta", "microphone", vec![]),
        ],
    };
    let prompt = build_prompt(&window);
    // First item body ends "...alpha"; the second begins "[a2]". Between them
    // there must be exactly a blank line — a "\n\n" boundary.
    let first = prompt.find("alpha").expect("first item body present");
    let second = prompt.find("[a2]").expect("second item header present");
    assert!(first < second, "items render in order");
    assert_eq!(
        &prompt[first + "alpha".len()..second],
        "\n\n",
        "items must be separated by exactly a blank line"
    );
}

#[test]
fn speaker_turns_can_never_carry_a_recognized_person_name() {
    // NAMES-NEVER-TO-CLOUD guard (ADR 0050). The prompt-builder's turn input is
    // `(cluster_id, text)` — there is NO name field — so a recognized person's
    // display name is UNREPRESENTABLE by construction: even if, on device, a
    // cluster maps to person "Jane Doe", only the anonymous label + the
    // already-redacted transcript text can ever be rendered. Here neither the
    // text nor the turns contain the sentinel, and the built prompt must not
    // either.
    const SENTINEL: &str = "Jane Doe";
    let turns = vec![(99, "thanks for the update".to_string())];
    let rendered = render_speaker_turns(&turns);
    assert!(rendered.contains("Speaker A: thanks for the update"));
    assert!(
        !rendered.contains(SENTINEL),
        "a name leaked into turns: {rendered}"
    );

    let window = CaptureWindow {
        start_ms: 0,
        end_ms: 10,
        items: vec![audio_item(
            1,
            5,
            "thanks for the update",
            "system_audio",
            turns,
        )],
    };
    let prompt = build_prompt(&window);
    assert!(!prompt.contains(SENTINEL), "a name leaked into the prompt");
    // The anonymous label is present; the identity is not.
    assert!(prompt.contains("Speaker A:"));
}

// === Recurrence Digest / local-clock prompt (Slice 3) ===================

fn distill_activity(
    id: i64,
    started_at_ms: i64,
    cat: Option<ActivityCategory>,
) -> capture_types::Activity {
    capture_types::Activity {
        id,
        title: format!("activity {id}"),
        summary: String::new(),
        category: cat,
        focus: None,
        started_at_ms,
        ended_at_ms: started_at_ms + 60_000,
        created_at_ms: started_at_ms,
        evidence: Vec::new(),
    }
}

#[test]
fn offset_label_formats_signed_hh_mm() {
    assert_eq!(offset_label(330), "UTC+05:30");
    assert_eq!(offset_label(-480), "UTC-08:00");
    assert_eq!(offset_label(0), "UTC+00:00");
}

#[test]
fn distillation_prompt_labels_times_local_with_offset() {
    let acts = vec![distill_activity(1, 0, Some(ActivityCategory::Creating))];
    let prompt = build_distillation_prompt(&acts, Some(330 * 60_000));
    assert!(
        prompt.contains("Times are local (UTC+05:30)."),
        "prompt: {prompt}"
    );
    // The rendered digits must actually shift, not just the label: the epoch
    // instant at +05:30 reads 1970-01-01 05:30 local.
    assert!(
        prompt.contains("[id=1] t=1970-01-01 05:30 UTC+05:30"),
        "prompt: {prompt}"
    );
    assert!(!prompt.contains("All times are UTC."));

    // A negative offset shifts backward across the day boundary.
    let prompt = build_distillation_prompt(&acts, Some(-480 * 60_000));
    assert!(
        prompt.contains("[id=1] t=1969-12-31 16:00 UTC-08:00"),
        "prompt: {prompt}"
    );
}

#[test]
fn distillation_prompt_falls_back_to_utc_without_offset() {
    let acts = vec![distill_activity(1, 0, Some(ActivityCategory::Creating))];
    let prompt = build_distillation_prompt(&acts, None);
    assert!(prompt.contains("All times are UTC."), "prompt: {prompt}");
    assert!(prompt.contains(" UTC\n") || prompt.contains(" UTC"));
}

#[test]
fn preamble_carries_the_observation_posture_invariant() {
    // Regression guard: the neutral-observation wording for focus routines must
    // survive verbatim (never a productivity judgment).
    assert!(conclusion_preamble().contains("neutral observation of the user's rhythm"));
}

#[test]
fn diarized_turns_cannot_blow_the_per_item_prompt_budget() {
    // ITEM_TEXT_CHAR_CAP caps the flat transcript "so a single noisy capture
    // cannot dominate the prompt budget". The diarized turns re-render the SAME
    // transcript beneath it; if they were uncapped, one long segment would
    // defeat the cap. A single long single-speaker segment: its flat text
    // truncates to ITEM_TEXT_CHAR_CAP, so its turn rendering must be bounded by
    // the same cap — the item's total transcript contribution stays within a
    // small multiple of the cap.
    let long = "lorem ipsum ".repeat(2_000); // ~24k chars
    let window = CaptureWindow {
        start_ms: 0,
        end_ms: 10,
        items: vec![audio_item(
            1,
            5,
            &long,
            "system_audio",
            vec![(1, long.clone())],
        )],
    };
    let prompt = build_prompt(&window);
    // Fixed intro/header + one item whose flat text AND turn rendering are each
    // capped at ITEM_TEXT_CHAR_CAP → at most a small multiple of the cap.
    let budget = 4 * ITEM_TEXT_CHAR_CAP;
    assert!(
        prompt.chars().count() <= budget,
        "one audio item produced a {}-char prompt (budget {budget}); diarized turns bypass ITEM_TEXT_CHAR_CAP",
        prompt.chars().count()
    );
}

// === C1: an empty extraction is a failure ===============================

fn derived(evidence_refs: &[&str]) -> DerivedActivity {
    DerivedActivity {
        title: "Edited the config".to_string(),
        summary: "Tuned the build.".to_string(),
        category: None,
        focus: None,
        evidence_refs: evidence_refs.iter().map(|r| r.to_string()).collect(),
        headline_ref: None,
        continues_id: None,
    }
}

fn completed_run() -> NewDerivationRun {
    NewDerivationRun {
        kind: "activity".to_string(),
        window_start_ms: Some(0),
        window_end_ms: Some(10),
        status: "completed".to_string(),
        activities_derived: 0,
        conclusions_derived: 0,
        input_tokens: 0,
        output_tokens: 0,
        provider: None,
        model: None,
        error: None,
        gate_drops: app_infra::DistillationGateDrops::default(),
    }
}

/// A batch that grounds nothing (no activities, or only refs the window never
/// showed) must be an `Err` so the worker records `failed` and retries it —
/// with neither Activities nor a `completed` run written; a grounded batch
/// writes both together.
#[tokio::test]
async fn a_batch_that_grounds_nothing_is_an_error() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("empty-extraction-{unique}"));
    std::fs::create_dir_all(&dir).expect("test dir");
    let infra = app_infra::AppInfra::initialize(&dir)
        .await
        .expect("app infra");
    let window = CaptureWindow {
        start_ms: 0,
        end_ms: 10,
        items: vec![audio_item(1, 5, "hello", "microphone", vec![])],
    };
    let tags = HashMap::from([("a1".to_string(), 5_i64)]);
    let neighbours = std::collections::HashSet::new();

    for batch in [vec![], vec![derived(&["f99", "a2"])]] {
        let batch = DerivedActivityBatch { activities: batch };
        let result = persist_batch(
            infra.user_context(),
            &window,
            &tags,
            &neighbours,
            &batch,
            completed_run(),
        )
        .await;
        assert!(
            result.is_err(),
            "ungrounded batch must fail, got {result:?}"
        );
    }
    let store = infra.user_context();
    assert_eq!(store.count_activities().await.expect("count"), 0);
    assert!(store
        .list_derivation_runs(10)
        .await
        .expect("runs")
        .is_empty());

    let grounded = DerivedActivityBatch {
        activities: vec![derived(&["a1"])],
    };
    let result = persist_batch(
        store,
        &window,
        &tags,
        &neighbours,
        &grounded,
        completed_run(),
    )
    .await;
    assert_eq!(result, Ok((1, 0)));
    assert_eq!(store.count_activities().await.expect("count"), 1);
    let runs = store.list_derivation_runs(10).await.expect("runs");
    assert_eq!(runs.len(), 1, "the window's run lands with its Activity");
    assert_eq!(
        (runs[0].status.as_str(), runs[0].activities_derived),
        ("completed", 1)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two Activities whose items interleave must not both claim the stretch:
/// the later one keeps only the time the first doesn't hold; one wholly inside
/// another's time is still its own row, nested; and two-frame Activities are
/// stored title-only.
#[tokio::test]
async fn interleaved_activities_in_one_window_do_not_overlap() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("interleaved-activities-{unique}"));
    std::fs::create_dir_all(&dir).expect("test dir");
    let infra = app_infra::AppInfra::initialize(&dir)
        .await
        .expect("app infra");
    let window = CaptureWindow {
        start_ms: 0,
        end_ms: 100,
        items: vec![],
    };
    let tags: HashMap<String, i64> = [("f1", 10), ("f2", 20), ("f3", 30), ("f4", 40), ("f5", 50)]
        .map(|(t, ms)| (t.to_string(), ms))
        .into();
    let batch = DerivedActivityBatch {
        activities: vec![
            derived(&["f2", "f4", "f5"]),
            derived(&["f1", "f3"]),
            derived(&["f2"]),
        ],
    };
    let result = persist_batch(
        infra.user_context(),
        &window,
        &tags,
        &std::collections::HashSet::new(),
        &batch,
        completed_run(),
    )
    .await;
    assert_eq!(result, Ok((3, 0)));
    let stored = infra
        .user_context()
        .list_activities_in_range(0, 100)
        .await
        .expect("list");
    let spans: Vec<(i64, i64, bool)> = stored
        .iter()
        .map(|a| (a.started_at_ms, a.ended_at_ms, a.summary.is_empty()))
        .collect();
    assert_eq!(spans, vec![(10, 30, true), (20, 20, true), (30, 50, false)]);
    assert_eq!(
        stored[1].title, "Edited the config",
        "the nested row keeps its own title"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
