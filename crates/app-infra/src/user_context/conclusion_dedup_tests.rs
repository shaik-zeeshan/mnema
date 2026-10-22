use super::*;
use crate::db::CaptureDb;
use crate::user_context::NewConclusion;

/// Synthetic fixtures modeled on the real-data report (one subject holding dozens
/// of rewordings of one sentence). Each pair is one belief reworded the way the
/// distiller does it: filler adverbs and "their"/"the" swaps.
const PARAPHRASES: &[(&str, &str)] = &[
    (
        "Unlocks the Mac at the login window several times a day",
        "Unlocks the Mac at the login window multiple times a day",
    ),
    (
        "Unlocks the Mac at the login window several times a day",
        "Frequently unlocks their Mac at the login window throughout the day",
    ),
    (
        "Unlocks the Mac at the login window several times a day",
        "Repeatedly unlocks the Mac via the login window during the day",
    ),
    (
        "Actively develops Mnema, a macOS screen-recording memory app",
        "Is actively developing Mnema, a macOS screen recording memory app",
    ),
];

/// Different beliefs about the SAME subject that a careless matcher fuses. The
/// first column of the comment is the measured content-token Jaccard (the guard
/// blocks the rest).
const DISTINCT: &[(&str, &str)] = &[
    // 0.22 + number/time guard
    (
        "Unlocks the Mac first thing in the morning around 9 AM",
        "Unlocks the Mac at the login window several times a day",
    ),
    // 0.22
    (
        "Uses Touch ID rather than a password to unlock the Mac",
        "Unlocks the Mac at the login window several times a day",
    ),
    // 0.60 + negation guard
    (
        "Plays Genshin Impact via a Windows VM",
        "Stopped playing Genshin Impact",
    ),
    // 0.75 + negation guard
    ("Plays Genshin Impact", "No longer plays Genshin Impact"),
    // 0.60 + time guard
    (
        "Plays Genshin Impact on weekends",
        "Plays Genshin Impact on weekdays",
    ),
    // 1.00 + number guard
    ("Works out 3 times a week", "Works out 5 times a week"),
    // 0.60
    (
        "Develops Mnema's onboarding flow",
        "Develops Mnema's licensing flow",
    ),
    // 0.67
    (
        "Uses Zed as their code editor",
        "Uses VS Code as their code editor",
    ),
    // 0.60
    (
        "Prefers dark mode in every app",
        "Prefers light mode in every app",
    ),
    // 0.67
    (
        "Checks Slack first thing in the morning",
        "Checks email first thing in the morning",
    ),
];

#[test]
fn lexical_rule_merges_paraphrases_and_keeps_distinct_beliefs() {
    for (a, b) in PARAPHRASES {
        assert!(
            is_lexical_duplicate(a, b),
            "paraphrase should merge: {a:?} ~ {b:?}"
        );
    }
    for (a, b) in DISTINCT {
        assert!(
            !is_lexical_duplicate(a, b),
            "distinct beliefs must not merge: {a:?} vs {b:?}"
        );
    }
    assert!(
        is_lexical_duplicate("Uses Rust", "uses rust "),
        "case-insensitive exact"
    );
}

#[test]
fn qualifier_guard_blocks_negation_number_and_time_swaps() {
    assert!(!statements_compatible("Plays X", "Doesn’t play X"));
    assert!(!statements_compatible("Runs 3 km", "Runs 5 km"));
    assert!(!statements_compatible(
        "Codes in the morning",
        "Codes in the evenings"
    ));
    assert!(statements_compatible(
        "Codes in the mornings",
        "Codes every morning"
    ));
}

#[test]
fn find_lexical_duplicate_picks_the_matching_row() {
    let existing = [
        (1, "Uses Touch ID rather than a password to unlock the Mac"),
        (2, "Unlocks the Mac at the login window several times a day"),
    ];
    assert_eq!(
        find_lexical_duplicate(
            "Frequently unlocks their Mac at the login window throughout the day",
            existing,
        ),
        Some(2)
    );
    assert_eq!(
        find_lexical_duplicate("Locks the Mac when away for lunch", existing),
        None
    );
}

async fn migrated_store() -> UserContextStore {
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
    crate::db::register_vec0_auto_extension();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory db should open");
    MIGRATOR.run(&pool).await.expect("migrations should apply");
    UserContextStore::new(CaptureDb::single(pool))
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(future)
}

async fn seed(
    store: &UserContextStore,
    subject: &str,
    statement: &str,
    confidence: f64,
    at: i64,
) -> i64 {
    store
        .upsert_conclusion(NewConclusion {
            subject: subject.to_string(),
            statement: statement.to_string(),
            confidence,
            formed_at_ms: at,
            last_supported_at_ms: at,
        })
        .await
        .expect("seed conclusion")
}

async fn activity(store: &UserContextStore) -> i64 {
    sqlx::query(
        "INSERT INTO user_context_activities (title, summary, started_at_ms, ended_at_ms, created_at_ms) \
         VALUES ('t', 's', 1, 2, 3) RETURNING id",
    )
    .fetch_one(store.db.write())
    .await
    .expect("activity")
    .get("id")
}

async fn link(store: &UserContextStore, conclusion: i64, activity: i64) {
    sqlx::query(
        "INSERT INTO user_context_conclusion_evidence (conclusion_id, activity_id, stance, created_at_ms) \
         VALUES (?1, ?2, 'support', 1)",
    )
    .bind(conclusion)
    .bind(activity)
    .execute(store.db.write())
    .await
    .expect("link");
}

async fn scalar(store: &UserContextStore, sql: &str) -> i64 {
    sqlx::query(sql)
        .fetch_one(store.db.read())
        .await
        .expect(sql)
        .get(0)
}

#[test]
fn heal_repoints_evidence_deletes_copies_and_is_idempotent() {
    block_on(async {
        let store = migrated_store().await;
        let subject = "Mac unlock routine";
        let keeper = seed(
            &store,
            subject,
            "Unlocks the Mac at the login window several times a day",
            0.9,
            100,
        )
        .await;
        let copy_tie = seed(
            &store,
            subject,
            "Unlocks the Mac at the login window multiple times a day",
            0.9,
            500,
        )
        .await;
        let copy_weak = seed(
            &store,
            "mac UNLOCK routine",
            "Frequently unlocks their Mac at the login window throughout the day",
            0.6,
            300,
        )
        .await;
        let distinct = seed(
            &store,
            subject,
            "Uses Touch ID rather than a password to unlock the Mac",
            0.7,
            200,
        )
        .await;
        // Same sentence under ANOTHER subject: never merged across subjects.
        let foreign = seed(
            &store,
            "Login window",
            "Unlocks the Mac at the login window several times a day",
            0.5,
            100,
        )
        .await;
        // A retired belief whose successor is one of the copies.
        let retired = seed(&store, subject, "Unlocks the Mac with a password", 0.2, 50).await;
        sqlx::query("UPDATE user_context_conclusions SET status = 'superseded', superseded_by = ?1 WHERE id = ?2")
            .bind(copy_tie)
            .bind(retired)
            .execute(store.db.write())
            .await
            .unwrap();

        let (a1, a2, a3) = (
            activity(&store).await,
            activity(&store).await,
            activity(&store).await,
        );
        link(&store, keeper, a1).await;
        link(&store, copy_tie, a1).await; // shared with the keeper: must not duplicate
        link(&store, copy_tie, a2).await;
        link(&store, copy_weak, a3).await;
        sqlx::query("INSERT INTO user_context_confidence_history (conclusion_id, confidence, snapshot_at_ms) VALUES (?1, 0.6, 1)")
            .bind(copy_weak)
            .execute(store.db.write())
            .await
            .unwrap();

        let summary = store
            .dedupe_near_duplicate_conclusions()
            .await
            .expect("heal");
        assert_eq!(
            summary,
            ConclusionDedupSummary {
                subjects_touched: 1,
                conclusions_removed: 2
            }
        );

        let ids: Vec<i64> = store
            .list_live_statements_for_subject(subject)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            ids,
            vec![keeper, distinct],
            "copies deleted, distinct belief kept"
        );
        assert_eq!(
            store
                .list_live_statements_for_subject("Login window")
                .await
                .unwrap()[0]
                .0,
            foreign
        );

        let mut evidence: Vec<i64> = sqlx::query(
            "SELECT activity_id FROM user_context_conclusion_evidence WHERE conclusion_id = ?1",
        )
        .bind(keeper)
        .fetch_all(store.db.read())
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get(0))
        .collect();
        evidence.sort();
        assert_eq!(evidence, vec![a1, a2, a3], "evidence re-pointed once each");
        assert_eq!(
            scalar(
                &store,
                "SELECT COUNT(*) FROM user_context_conclusion_evidence"
            )
            .await,
            3
        );
        assert_eq!(
            scalar(&store, "SELECT COUNT(*) FROM user_context_confidence_history WHERE conclusion_id NOT IN (SELECT id FROM user_context_conclusions)").await,
            0,
            "no orphaned history",
        );
        assert_eq!(
            scalar(
                &store,
                &format!("SELECT superseded_by FROM user_context_conclusions WHERE id = {retired}")
            )
            .await,
            keeper,
            "superseded_by re-pointed to the keeper",
        );
        assert_eq!(
            scalar(
                &store,
                &format!(
                    "SELECT last_supported_at_ms FROM user_context_conclusions WHERE id = {keeper}"
                )
            )
            .await,
            500,
            "keeper inherits the latest support",
        );

        let again = store
            .dedupe_near_duplicate_conclusions()
            .await
            .expect("re-run");
        assert_eq!(again, ConclusionDedupSummary::default(), "idempotent");
    });
}

#[test]
fn heal_never_removes_a_pinned_copy() {
    block_on(async {
        let store = migrated_store().await;
        let strong = seed(&store, "cmux", "Uses cmux as the main terminal", 0.9, 1).await;
        let pinned = seed(
            &store,
            "cmux",
            "Uses cmux as the main terminal regularly",
            0.5,
            2,
        )
        .await;
        sqlx::query("UPDATE user_context_conclusions SET pinned = 1 WHERE id = ?1")
            .bind(pinned)
            .execute(store.db.write())
            .await
            .unwrap();

        let summary = store
            .dedupe_near_duplicate_conclusions()
            .await
            .expect("heal");
        assert_eq!(summary.conclusions_removed, 1);
        let ids: Vec<i64> = store
            .list_live_statements_for_subject("cmux")
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            ids,
            vec![pinned],
            "the pinned row is the keeper; {strong} folded into it"
        );
        let confidence: f64 =
            sqlx::query("SELECT confidence FROM user_context_conclusions WHERE id = ?1")
                .bind(pinned)
                .fetch_one(store.db.read())
                .await
                .unwrap()
                .get(0);
        assert_eq!(
            confidence, 0.9,
            "the keeper takes the stronger copy's confidence"
        );
    });
}

#[test]
fn known_subjects_listing_mixes_strongest_and_newest_without_paraphrases() {
    block_on(async {
        let store = migrated_store().await;
        // Old saturated rows (two of them rewordings of one belief) + one fresh belief.
        let old = seed(&store, "Mnema", "Develops Mnema's onboarding flow", 0.9, 10).await;
        let old_copy = seed(
            &store,
            "Mnema",
            "Actively develops Mnema's onboarding flow",
            0.9,
            11,
        )
        .await;
        let old_other = seed(&store, "Mnema", "Develops Mnema's licensing flow", 0.9, 12).await;
        let fresh = seed(
            &store,
            "Mnema",
            "Profiles Mnema's embedding worker for heat",
            0.5,
            99,
        )
        .await;

        let shown: Vec<i64> = store
            .list_conclusions_for_subjects(&["mnema".to_string()], 3)
            .await
            .unwrap()
            .into_iter()
            .map(|(_, id, ..)| id)
            .collect();
        assert_eq!(
            shown,
            vec![old, fresh, old_other],
            "strongest + newest, paraphrase {old_copy} skipped"
        );
    });
}
