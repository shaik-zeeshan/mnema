use sqlx::Row;

use super::*;
use crate::db::CaptureDb;
use crate::user_context::store::CONFIDENCE_DECAY_MIN_INTERVAL_MS;
use crate::user_context::NewConclusion;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(future)
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

async fn seed(store: &UserContextStore, statement: &str, supported_at: i64) -> i64 {
    store
        .upsert_conclusion(NewConclusion {
            subject: "Rust".to_string(),
            statement: statement.to_string(),
            confidence: 0.8,
            formed_at_ms: supported_at,
            last_supported_at_ms: supported_at,
        })
        .await
        .expect("seed conclusion")
}

async fn history_rows(store: &UserContextStore) -> i64 {
    sqlx::query("SELECT COUNT(*) FROM user_context_confidence_history")
        .fetch_one(store.db.read())
        .await
        .expect("count")
        .get(0)
}

#[test]
fn only_conclusions_past_the_minimum_interval_are_due() {
    block_on(async {
        let store = migrated_store().await;
        let now = 100 * DAY_MS;
        let fresh = seed(&store, "Learning Rust", now - CONFIDENCE_DECAY_MIN_INTERVAL_MS + 1).await;
        let due = seed(&store, "Writes async Rust", now - CONFIDENCE_DECAY_MIN_INTERVAL_MS).await;

        let ids: Vec<i64> = store
            .list_decayable_conclusions(now)
            .await
            .expect("decayable")
            .iter()
            .map(|c| c.conclusion.id)
            .collect();
        assert_eq!(ids, vec![due], "exactly 24 h is due; {fresh} (just under) is not");
    });
}

#[test]
fn a_pass_writes_every_row_in_one_call_and_a_repeat_writes_nothing() {
    block_on(async {
        let store = migrated_store().await;
        let now = 100 * DAY_MS;
        let a = seed(&store, "Learning Rust", now - 3 * DAY_MS).await;
        let b = seed(&store, "Writes async Rust", now - 2 * DAY_MS).await;
        let baseline = history_rows(&store).await;

        let due = store.list_decayable_conclusions(now).await.expect("decayable");
        assert_eq!(due.len(), 2);
        let decayed: Vec<_> = due
            .iter()
            .map(|c| (c.conclusion.id, 0.1, ConclusionStatus::Faded))
            .collect();
        store
            .apply_confidence_decay_pass(&decayed, now, 64)
            .await
            .expect("pass");

        for id in [a, b] {
            let row = sqlx::query(
                "SELECT confidence, status, last_decayed_at_ms FROM user_context_conclusions WHERE id = ?1",
            )
            .bind(id)
            .fetch_one(store.db.read())
            .await
            .expect("row");
            assert_eq!(row.get::<f64, _>("confidence"), 0.1);
            assert_eq!(row.get::<String, _>("status"), "faded");
            assert_eq!(row.get::<i64, _>("last_decayed_at_ms"), now);
        }
        assert_eq!(history_rows(&store).await, baseline + 2, "one snapshot per row");

        // Same `now` again: the anchors just moved to `now`, so nothing is due and
        // the (empty) pass writes nothing.
        let again = store.list_decayable_conclusions(now).await.expect("decayable");
        assert!(again.is_empty(), "nothing is due right after a pass");
        store
            .apply_confidence_decay_pass(&[], now, 64)
            .await
            .expect("empty pass");
        assert_eq!(history_rows(&store).await, baseline + 2, "no extra snapshots");
    });
}
