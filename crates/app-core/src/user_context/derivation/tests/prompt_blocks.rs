//! Dismissal, authored-context and known-subject prompt-block tests.

use super::*;

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
    let items: Vec<AuthoredContext> = (0..50).map(|i| authored(i, &long, None)).collect();
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
        authored_prompt_lines(items)
            .iter()
            .map(Option::is_some)
            .collect()
    };
    // Newest is always included, even alone over the cap.
    let huge = "x".repeat(AUTHORED_CONTEXT_CHAR_CAP + 50);
    assert_eq!(
        in_prompt(&[authored(1, &huge, None), authored(0, "short", None)]),
        [true, false]
    );
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
    assert_eq!(
        in_prompt(&[authored(2, "  ", None), authored(1, "a", None)]),
        [false, true]
    );
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
    let infra = app_infra::AppInfra::initialize(&dir)
        .await
        .expect("app infra");
    let store = infra.user_context();

    let half = "z".repeat(AUTHORED_CONTEXT_CHAR_CAP / 2 + 100);
    let edited = store
        .add_authored_context(&half, None, 1_000)
        .await
        .expect("add");
    let untouched = store
        .add_authored_context(&half, None, 2_000)
        .await
        .expect("add");
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
    assert!(
        block.contains("    1: xxx"),
        "first subject's conclusion shown"
    );
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
    let d = dismissal(
        "Apple",
        "Interested in Apple",
        &evidence_fingerprint(&[1, 2]),
        2,
    );
    let dismissals = vec![d];

    // Same evidence as rejected → must be recognized as a same-evidence match.
    let same = evidence_fingerprint(&[2, 1]);
    let matched = matching_dismissal(&dismissals, "apple", "interested in apple").expect("matches");
    assert_eq!(same, matched.evidence_fingerprint, "same evidence => drop");

    // Different (more) evidence: fingerprint differs, and the resurface bar
    // gates on the support count vs the prior 2 (needs ≥ 4 at 2.0×).
    assert_ne!(
        evidence_fingerprint(&[1, 2, 3, 4]),
        matched.evidence_fingerprint
    );
    assert!(!confidence::meets_resurface_bar(
        3,
        matched.evidence_activity_count
    ));
    assert!(confidence::meets_resurface_bar(
        4,
        matched.evidence_activity_count
    ));
}

#[test]
fn distillation_uses_the_confidence_policy_formation_bar() {
    // The formation bar (#95) gates distillation: a single supporting
    // Activity is below the bar and must not form a Conclusion.
    assert!(!confidence::meets_formation_bar(1));
    assert!(confidence::meets_formation_bar(2));
    // And the wired initial_confidence rises with support, drops on contradiction.
    assert!(confidence::initial_confidence(3, 0) > confidence::initial_confidence(2, 0));
    assert!(confidence::initial_confidence(3, 1) < confidence::initial_confidence(3, 0));
}
