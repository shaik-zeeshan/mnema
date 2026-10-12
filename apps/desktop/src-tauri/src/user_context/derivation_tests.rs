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
        assert_eq!(parse_category(&Some(raw.to_string())), Some(expected), "{raw}");
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
        assert_eq!(parse_category(&Some(raw.to_string())), Some(expected), "{raw}");
    }
}

#[test]
fn parses_known_focus_levels_only() {
    assert_eq!(parse_focus(&Some("Deep".to_string())), Some(FocusLevel::Deep));
    assert_eq!(parse_focus(&Some(" mixed ".to_string())), Some(FocusLevel::Mixed));
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
            correction(i, &long, Some(ActivityCategory::Creating), Some(FocusLevel::Deep))
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

#[test]
fn matching_dismissal_is_case_insensitive_on_subject_and_statement() {
    let dismissals = vec![dismissal("Apple", "Interested in Apple", "1,2", 2)];
    // Exact match on both, case-insensitively.
    assert!(matching_dismissal(&dismissals, "apple", "INTERESTED IN APPLE").is_some());
    // A different statement (even same subject) does not match.
    assert!(matching_dismissal(&dismissals, "Apple", "Loves Apple").is_none());
    // A different subject does not match.
    assert!(matching_dismissal(&dismissals, "Rust", "Interested in Apple").is_none());
}

fn authored(id: i64, text: &str, topic: Option<&str>) -> AuthoredContext {
    AuthoredContext {
        id,
        text: text.to_string(),
        topic: topic.map(str::to_string),
        created_at_ms: 1_000,
        updated_at_ms: 1_000,
    }
}

#[test]
fn authored_context_block_is_empty_without_statements() {
    assert!(build_authored_context_block(&[]).is_empty());
    // All-blank statements also yield no block.
    assert!(build_authored_context_block(&[authored(1, "   ", None)]).is_empty());
}

#[test]
fn authored_context_block_labels_statements_and_topics() {
    let items = vec![
        authored(2, "I care about typography", None),
        authored(1, "I'm a designer", Some("role")),
    ];
    let block = build_authored_context_block(&items);
    assert!(block.contains("USER-AUTHORED CONTEXT"));
    assert!(block.contains("ASSERTED by the user"));
    assert!(block.contains("- I care about typography"));
    assert!(block.contains("- (topic: role) I'm a designer"));
    // Never cited as Activity evidence.
    assert!(block.contains("do not cite them as support_refs"));
}

#[test]
fn authored_context_block_respects_char_cap() {
    // Many long statements; the block must stay bounded by the cap (plus the
    // single header + one over-budget line that the !lines.is_empty() guard
    // allows once).
    let long = "x".repeat(300);
    let items: Vec<AuthoredContext> = (0..50)
        .map(|i| authored(i, &long, None))
        .collect();
    let block = build_authored_context_block(&items);
    // The header is always present; the statement body stays near the cap and
    // does not include all 50 * 300 chars.
    assert!(block.contains("USER-AUTHORED CONTEXT"));
    assert!(
        block.chars().count() < AUTHORED_CONTEXT_CHAR_CAP + 600,
        "block stays bounded by the char cap (+header/one-line slack)"
    );
}

#[test]
fn authored_in_prompt_follows_the_distillation_counting_rule() {
    let in_prompt = |items: &[AuthoredContext]| -> Vec<bool> {
        authored_prompt_lines(items).iter().map(Option::is_some).collect()
    };
    // Newest is always included, even alone over the cap.
    let huge = "x".repeat(AUTHORED_CONTEXT_CHAR_CAP + 50);
    assert_eq!(in_prompt(&[authored(1, &huge, None), authored(0, "short", None)]), [true, false]);
    // Stop at the first overflow: a short older statement that WOULD fit after
    // the overflowing one is still out.
    let big = "y".repeat(AUTHORED_CONTEXT_CHAR_CAP / 2 + 100);
    let items = [
        authored(3, &big, None),
        authored(2, &big, None),
        authored(1, "tiny", None),
    ];
    assert_eq!(in_prompt(&items), [true, false, false]);
    // The block contains exactly the in-prompt lines.
    let block = build_authored_context_block(&items);
    assert_eq!(block.matches(&big).count(), 1);
    assert!(!block.contains("tiny"));
    // Blank statements are never read.
    assert_eq!(in_prompt(&[authored(2, "  ", None), authored(1, "a", None)]), [false, true]);
}

/// At the cap, a statement edited today is read before an older untouched
/// one, even though it was created first (store order is `updated_at_ms`).
#[tokio::test]
async fn an_edited_statement_is_included_before_an_older_untouched_one_at_the_cap() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("authored-cap-order-{unique}"));
    std::fs::create_dir_all(&dir).expect("test dir");
    let infra = app_infra::AppInfra::initialize(&dir).await.expect("app infra");
    let store = infra.user_context();

    let half = "z".repeat(AUTHORED_CONTEXT_CHAR_CAP / 2 + 100);
    let edited = store.add_authored_context(&half, None, 1_000).await.expect("add");
    let untouched = store.add_authored_context(&half, None, 2_000).await.expect("add");
    store
        .update_authored_context(edited, &half, Some("Work"), 3_000)
        .await
        .expect("edit");

    let listed = store.list_authored_context().await.expect("list");
    let read: Vec<(i64, bool)> = listed
        .iter()
        .zip(authored_prompt_lines(&listed))
        .map(|(item, line)| (item.id, line.is_some()))
        .collect();
    assert_eq!(read, [(edited, true), (untouched, false)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dismissed_conclusions_block_is_empty_without_dismissals() {
    assert!(build_dismissed_conclusions_block(&[]).is_empty());
}

#[test]
fn dismissed_conclusions_block_lists_each_rejection() {
    let dismissals = vec![
        dismissal("Apple", "Interested in Apple", "1,2", 2),
        dismissal("Vim", "Prefers Vim", "3,4", 2),
    ];
    let block = build_dismissed_conclusions_block(&dismissals);
    assert!(block.contains("DISMISSED CONCLUSIONS"));
    assert!(block.contains("subject=Apple statement=Interested in Apple"));
    assert!(block.contains("subject=Vim statement=Prefers Vim"));
    // The instruction enforces the high-bar / never-same-evidence rule.
    assert!(block.contains("substantially MORE and NEWER"));
}

#[test]
fn known_subjects_block_is_empty_without_handles() {
    let empty: std::collections::HashMap<String, Vec<(i64, String)>> = HashMap::new();
    assert!(build_known_subjects_block(&[], &empty).is_empty());
    // All-blank handles also yield no block (prompt unchanged).
    assert!(build_known_subjects_block(&["   ".to_string()], &empty).is_empty());
}

#[test]
fn known_subjects_block_renders_handles_and_header() {
    let handles = vec!["Apple".to_string(), "async communication".to_string()];
    let empty: std::collections::HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let block = build_known_subjects_block(&handles, &empty);
    assert!(block.contains("KNOWN SUBJECTS"));
    // The reuse-verbatim instruction frames the block.
    assert!(block.contains("VERBATIM"));
    assert!(block.contains("reinforces the existing subject"));
    // One `- {handle}` line per handle, in order.
    assert!(block.contains("- Apple"));
    assert!(block.contains("- async communication"));
}

#[test]
fn known_subjects_block_nests_conclusions_in_order() {
    let handles = vec!["Apple".to_string()];
    let mut map: std::collections::HashMap<String, Vec<(i64, String)>> = HashMap::new();
    // Confidence-desc order is the caller's job; the block preserves push order.
    map.insert(
        "Apple".to_string(),
        vec![
            (7, "Interested in Apple silicon".to_string()),
            (9, "Follows Apple keynotes".to_string()),
        ],
    );
    let block = build_known_subjects_block(&handles, &map);
    assert!(block.contains("- Apple\n"));
    // Indented `id: statement` lines nested under the handle, in order.
    let first = block.find("    7: Interested in Apple silicon").unwrap();
    let second = block.find("    9: Follows Apple keynotes").unwrap();
    assert!(first < second, "conclusion lines keep their given order");
    // The instruction explains the reinforces-by-id semantics.
    assert!(block.contains("reinforces_id"));
}

#[test]
fn known_subjects_block_degrades_overflowing_subject_to_handle_only() {
    // Two subjects, each with one very long conclusion line. Sized so the first
    // subject's full block fits but the second's conclusion line overflows the cap,
    // forcing the second down to handle-only.
    let long = "x".repeat(KNOWN_SUBJECTS_CHAR_CAP * 3 / 4);
    let handles = vec!["First".to_string(), "Second".to_string()];
    let mut map: std::collections::HashMap<String, Vec<(i64, String)>> = HashMap::new();
    map.insert("First".to_string(), vec![(1, long.clone())]);
    map.insert("Second".to_string(), vec![(2, long)]);
    let block = build_known_subjects_block(&handles, &map);
    // First subject keeps its conclusion line...
    assert!(block.contains("    1: xxx"), "first subject's conclusion shown");
    // ...but the second degrades to handle-only (its `2:` line dropped).
    assert!(block.contains("- Second\n"), "second handle still present");
    assert!(
        !block.contains("    2: "),
        "overflowing subject degrades to handle-only"
    );
}

#[test]
fn known_subjects_block_respects_char_cap() {
    // Many handles; the block stays bounded by the cap (plus the header + the
    // single over-budget line the lines.is_empty() guard allows once).
    let handles: Vec<String> = (0..2_000).map(|i| format!("subject-{i:05}")).collect();
    let empty: std::collections::HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let block = build_known_subjects_block(&handles, &empty);
    assert!(block.contains("KNOWN SUBJECTS"));
    assert!(
        block.chars().count() < KNOWN_SUBJECTS_CHAR_CAP + 700,
        "block stays bounded by the char cap (+header/one-line slack)"
    );
}

#[test]
fn conclusion_preamble_instructs_reuse_of_known_subject_handles() {
    let preamble = conclusion_preamble();
    assert!(preamble.contains("KNOWN SUBJECTS"));
    assert!(preamble.contains("reuse that handle VERBATIM"));
    assert!(preamble.contains("genuinely new subject"));
}

#[test]
fn resurface_gate_drops_same_evidence_and_below_bar() {
    // The fresh fingerprint for the same evidence set matches the dismissal's,
    // so the same-evidence drop fires regardless of count.
    let d = dismissal("Apple", "Interested in Apple", &evidence_fingerprint(&[1, 2]), 2);
    let dismissals = vec![d];

    // Same evidence as rejected → must be recognized as a same-evidence match.
    let same = evidence_fingerprint(&[2, 1]);
    let matched = matching_dismissal(&dismissals, "apple", "interested in apple")
        .expect("matches");
    assert_eq!(same, matched.evidence_fingerprint, "same evidence => drop");

    // Different (more) evidence: fingerprint differs, and the resurface bar
    // gates on the support count vs the prior 2 (needs ≥ 4 at 2.0×).
    assert_ne!(evidence_fingerprint(&[1, 2, 3, 4]), matched.evidence_fingerprint);
    assert!(!confidence::meets_resurface_bar(3, matched.evidence_activity_count));
    assert!(confidence::meets_resurface_bar(4, matched.evidence_activity_count));
}

#[test]
fn distillation_uses_the_confidence_policy_formation_bar() {
    // The formation bar (#95) gates distillation: a single supporting
    // Activity is below the bar and must not form a Conclusion.
    assert!(!confidence::meets_formation_bar(1));
    assert!(confidence::meets_formation_bar(2));
    // And the wired initial_confidence rises with support, drops on contradiction.
    assert!(
        confidence::initial_confidence(3, 0) > confidence::initial_confidence(2, 0)
    );
    assert!(
        confidence::initial_confidence(3, 1) < confidence::initial_confidence(3, 0)
    );
}

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
    assert_eq!(source_tag("audio_segment", Some("system_audio")), Some("computer"));
    // Unknown / absent source, and any non-audio subject, get no tag.
    assert_eq!((source_tag("audio_segment", None), source_tag("audio_segment", Some("bogus"))), (None, None));
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
    assert!(prompt.contains("never render computer audio as words the user said") && prompt.contains("it is a conversation only"));
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
    assert!(!rendered.contains(SENTINEL), "a name leaked into turns: {rendered}");

    let window = CaptureWindow {
        start_ms: 0,
        end_ms: 10,
        items: vec![audio_item(1, 5, "thanks for the update", "system_audio", turns)],
    };
    let prompt = build_prompt(&window);
    assert!(!prompt.contains(SENTINEL), "a name leaked into the prompt");
    // The anonymous label is present; the identity is not.
    assert!(prompt.contains("Speaker A:"));
}

// === Recurrence Digest / local-clock prompt (Slice 3) ===================

fn distill_activity(id: i64, started_at_ms: i64, cat: Option<ActivityCategory>) -> capture_types::Activity {
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
    assert!(prompt.contains("Times are local (UTC+05:30)."), "prompt: {prompt}");
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
        items: vec![audio_item(1, 5, &long, "system_audio", vec![(1, long.clone())])],
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
