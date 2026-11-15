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

async fn snapshots(store: &UserContextStore, id: i64) -> Vec<(f64, i64)> {
    store
        .list_confidence_history(id)
        .await
        .expect("history")
        .iter()
        .map(|s| (s.confidence, s.snapshot_at_ms))
        .collect()
}

async fn decay_columns(store: &UserContextStore, id: i64) -> (f64, String, Option<i64>) {
    let row = sqlx::query(
        "SELECT confidence, status, last_decayed_at_ms FROM user_context_conclusions WHERE id = ?1",
    )
    .bind(id)
    .fetch_one(store.db.read())
    .await
    .expect("row");
    (row.get("confidence"), row.get("status"), row.get("last_decayed_at_ms"))
}

#[test]
fn rows_dismissed_or_pinned_after_the_read_are_skipped_not_fatal() {
    block_on(async {
        let store = migrated_store().await;
        let now = 100 * DAY_MS;
        let kept_a = seed(&store, "Learning Rust", now - 3 * DAY_MS).await;
        let kept_b = seed(&store, "Writes async Rust", now - 3 * DAY_MS).await;
        let dismissed = seed(&store, "Reads Rust books", now - 3 * DAY_MS).await;
        let pinned = seed(&store, "Uses cargo", now - 3 * DAY_MS).await;

        let due = store.list_decayable_conclusions(now).await.expect("decayable");
        assert_eq!(due.len(), 4);
        // Distinct values per row, so a swapped binding would show.
        let decayed: Vec<_> = [(kept_a, 0.11), (kept_b, 0.22), (dismissed, 0.33), (pinned, 0.44)]
            .into_iter()
            .map(|(id, c)| (id, c, ConclusionStatus::Faded))
            .collect();

        // Changed between the read and the write.
        store.dismiss_conclusion(dismissed).await.expect("dismiss");
        store.set_pinned(pinned, true).await.expect("pin");
        let pinned_before = decay_columns(&store, pinned).await;
        let pinned_history = snapshots(&store, pinned).await;

        let applied = store
            .apply_confidence_decay_pass(&decayed, now, 64)
            .await
            .expect("a vanished row must not fail the pass");
        assert_eq!(applied, 2, "only the two untouched rows decay");

        for (id, c) in [(kept_a, 0.11), (kept_b, 0.22)] {
            assert_eq!(decay_columns(&store, id).await, (c, "faded".to_string(), Some(now)));
            assert_eq!(snapshots(&store, id).await.last(), Some(&(c, now)), "snapshot of {id}");
        }
        let orphaned: i64 = sqlx::query(
            "SELECT COUNT(*) FROM user_context_confidence_history WHERE conclusion_id = ?1",
        )
        .bind(dismissed)
        .fetch_one(store.db.read())
        .await
        .expect("count")
        .get(0);
        assert_eq!(orphaned, 0, "no snapshot for the dismissed row");
        assert_eq!(decay_columns(&store, pinned).await, pinned_before, "pinned row untouched");
        assert_eq!(snapshots(&store, pinned).await, pinned_history, "pinned row gets no snapshot");
    });
}

#[test]
fn the_pass_caps_history_per_conclusion_across_the_whole_table() {
    block_on(async {
        let store = migrated_store().await;
        let now = 100 * DAY_MS;
        let a = seed(&store, "Learning Rust", now - 3 * DAY_MS).await;
        let b = seed(&store, "Writes async Rust", now - 3 * DAY_MS).await;
        sqlx::query("DELETE FROM user_context_confidence_history")
            .execute(store.db.write())
            .await
            .expect("clear seed snapshots");
        for id in [a, b] {
            for t in [now - 3_000, now - 2_000, now - 1_000] {
                store.insert_confidence_snapshot(id, 0.5, t).await.expect("snap");
            }
        }

        let applied = store
            .apply_confidence_decay_pass(&[(a, 0.4, ConclusionStatus::Visible)], now, 2)
            .await
            .expect("pass");
        assert_eq!(applied, 1);
        assert_eq!(snapshots(&store, a).await, vec![(0.5, now - 1_000), (0.4, now)]);
        assert_eq!(
            snapshots(&store, b).await,
            vec![(0.5, now - 2_000), (0.5, now - 1_000)],
            "the cap applies to every Conclusion, not just this pass's"
        );
    });
}

#[test]
fn a_future_anchor_heals_in_one_pass_and_the_next_is_due_a_day_later() {
    block_on(async {
        let store = migrated_store().await;
        let now = 100 * DAY_MS;
        let past = seed(&store, "Learning Rust", now - 3 * DAY_MS).await;
        // Stamped while the clock was ahead: must still be listed, not frozen.
        let future = seed(&store, "Writes async Rust", now + 5 * DAY_MS).await;

        let due = store.list_decayable_conclusions(now).await.expect("decayable");
        let mut ids: Vec<i64> = due.iter().map(|c| c.conclusion.id).collect();
        ids.sort();
        assert_eq!(ids, vec![past, future]);
        let decayed: Vec<_> = due
            .iter()
            .map(|c| {
                let value = crate::user_context::confidence::decay(
                    c.conclusion.confidence,
                    c.decay_anchor_ms,
                    now,
                    false,
                );
                (c.conclusion.id, value, ConclusionStatus::Visible)
            })
            .collect();
        store
            .apply_confidence_decay_pass(&decayed, now, 64)
            .await
            .expect("pass");
        assert_eq!(
            decay_columns(&store, future).await,
            (0.8, "visible".to_string(), Some(now)),
            "unchanged value, anchor pulled back to now"
        );

        let just_under = now + CONFIDENCE_DECAY_MIN_INTERVAL_MS - 1;
        assert!(store.list_decayable_conclusions(just_under).await.expect("decayable").is_empty());
        let at_interval = now + CONFIDENCE_DECAY_MIN_INTERVAL_MS;
        assert_eq!(
            store.list_decayable_conclusions(at_interval).await.expect("decayable").len(),
            2,
            "both are due exactly one interval after the pass"
        );
    });
}
