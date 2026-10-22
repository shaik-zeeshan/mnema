//! Scale check for one **Semantic Index Backfill** worker pass on a ~1.4 GB
//! Encrypted Capture Index.
//!
//! The 2026-10-17 `fs_usage` trace of v0.1.23 had the app re-reading its own
//! 970 MB database ~15× in 23 minutes, ~210 MB per worker pass. This seeds a
//! database of that shape through the app's own pool setup (SQLCipher key,
//! `vec0` auto-extension, Owner Writer pragmas) and counts the pages each
//! statement of a pass pulls through the pager on a cold cache — which is what
//! the live app pays, because every capture commit changes the WAL and resets
//! every reader's page cache.
//!
//! `#[ignore]`d because seeding takes about a minute; run it explicitly:
//!
//! ```text
//! cargo test -p app-infra --release --lib semantic_backfill_scale -- --ignored --nocapture
//! ```

use std::future::Future;

use sqlx::{Row, SqlitePool};

use super::test_support::{run_async_test, test_dir};
use crate::db::{connect_single_connection_owner_pool, CaptureDb};
use crate::semantic_search::{SemanticSearchStore, ANCHORS_MISSING_VECTOR_SQL};

/// `search_documents` rows. Half are `direct` with ~1.4 KB of OCR text (two
/// to a page), half `equivalent_reuse` (NULL body): ~1.4 GB on disk with the
/// vectors. Sized so the old anti-join pass costs about what the 2026-10-17
/// trace saw: 917k reads over 71 passes, ~13k per pass.
const DOCUMENTS: i64 = 600_000;
/// Newest `direct` anchors left without a vector: the live-capture backlog. Live
/// passes embed 1–9 anchors (rust.log, 2026-10-17), i.e. fewer than the
/// worker's batch of 16, so the peek's `LIMIT` never stops it early.
const BACKLOG: i64 = 6;
const BATCH: i64 = 16;
const MODEL_ID: &str = "nomic-embed-text-v1.5";
const DIMENSION: usize = 768;

async fn seed(pool: &SqlitePool, store: &SemanticSearchStore) {
    store
        .recreate_vectors_table(MODEL_ID, DIMENSION)
        .await
        .expect("vector table should be created and stamped");
    // A big cache for the seeding transaction only; the measured connection is
    // reset back to the app default below.
    sqlx::query("PRAGMA cache_size = -262144")
        .execute(pool)
        .await
        .expect("cache_size");
    let mut tx = pool.begin().await.expect("tx should begin");
    sqlx::query(
        "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?1) \
         INSERT INTO frames (id, session_id, file_path, captured_at) \
         SELECT n, 'seed-session', '/tmp/seed-' || n || '.jpg', '2026-10-17T10:00:00Z' FROM seq",
    )
    .bind(DOCUMENTS)
    .execute(&mut *tx)
    .await
    .expect("frames should insert");
    // Odd ids are `direct` (random hex: incompressible, like real OCR pages are
    // to the pager); each even id reuses the `direct` row before it. Time rises
    // with id, as capture does.
    sqlx::query(
        "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?1) \
         INSERT INTO search_documents (\
             id, anchor_type, frame_id, absolute_start_at, absolute_end_at, session_id, \
             app_name, window_title, group_key, text_source_kind, body_text, \
             canonical_search_document_id, context_text) \
         SELECT n, 'frame', n, \
                strftime('%Y-%m-%dT%H:%M:%fZ', 1760000000 + n * 2, 'unixepoch'), \
                strftime('%Y-%m-%dT%H:%M:%fZ', 1760000001 + n * 2, 'unixepoch'), \
                'seed-session', 'Safari', 'Pull requests', 'frame:' || n, \
                CASE n % 2 WHEN 1 THEN 'direct' ELSE 'equivalent_reuse' END, \
                CASE n % 2 WHEN 1 THEN 'ocr ' || n || ' ' || hex(randomblob(700)) END, \
                CASE n % 2 WHEN 1 THEN NULL ELSE n - 1 END, \
                'Safari' FROM seq",
    )
    .bind(DOCUMENTS)
    .execute(&mut *tx)
    .await
    .expect("documents should insert");
    // Every `direct` anchor but the newest BACKLOG is already vectored.
    sqlx::query(
        "INSERT INTO search_document_vectors (rowid, embedding) \
         SELECT id, vec_int8(randomblob(?2)) FROM search_documents \
         WHERE text_source_kind = 'direct' AND id <= ?1 - ?3 * 2 ORDER BY id",
    )
    .bind(DOCUMENTS)
    .bind(DIMENSION as i64)
    .bind(BACKLOG)
    .execute(&mut *tx)
    .await
    .expect("vectors should insert");
    // The store's own bookkeeping for those vectors.
    sqlx::query("DELETE FROM search_document_vector_backlog WHERE id <= ?1 - ?2 * 2")
        .bind(DOCUMENTS)
        .bind(BACKLOG)
        .execute(&mut *tx)
        .await
        .expect("backlog should shrink");
    tx.commit().await.expect("seed should commit");
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
        .expect("checkpoint");
    // Back to SQLCipher's default page cache, the one the app's readers run with.
    sqlx::query("PRAGMA cache_size = -2000")
        .execute(pool)
        .await
        .expect("cache_size");
}

/// Pages the pool's single connection pulls through the pager while `step` runs,
/// starting from an empty page cache.
async fn cold_pages<T, Fut: Future<Output = T>>(pool: &SqlitePool, step: Fut) -> (T, i64) {
    pager_counter(pool, true).await;
    let out = step.await;
    (out, pager_counter(pool, false).await)
}

async fn pager_counter(pool: &SqlitePool, reset: bool) -> i64 {
    let mut connection = pool.acquire().await.expect("connection");
    let mut handle = connection.lock_handle().await.expect("handle");
    let db = handle.as_raw_handle().as_ptr();
    let (mut current, mut high) = (0, 0);
    // SAFETY: `db` is the live handle of a connection held for this whole block.
    unsafe {
        if reset {
            libsqlite3_sys::sqlite3_db_release_memory(db);
        }
        libsqlite3_sys::sqlite3_db_status(
            db,
            libsqlite3_sys::SQLITE_DBSTATUS_CACHE_MISS,
            &mut current,
            &mut high,
            i32::from(reset),
        );
    }
    i64::from(current)
}

async fn explain(pool: &SqlitePool, sql: &str) -> String {
    sqlx::query(&format!("EXPLAIN QUERY PLAN {sql}"))
        .bind(BATCH)
        .fetch_all(pool)
        .await
        .expect("explain should run")
        .into_iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The pre-0059 shape of the peek / re-check / count, kept to measure what the
/// backlog replaced. Never ship it: see the module docs.
const VEC0_ANTI_JOIN: &str = "search_documents.text_source_kind = 'direct' \
     AND search_documents.id NOT IN (SELECT rowid FROM search_document_vectors)";

async fn vec0_anti_join_pass_pages(pool: &SqlitePool, ids: &[i64]) -> (i64, i64, i64) {
    let peek = format!(
        "SELECT id, body_text FROM search_documents WHERE {VEC0_ANTI_JOIN} \
         ORDER BY absolute_start_at DESC, id DESC LIMIT ?1"
    );
    let (rows, peek_pages) = cold_pages(pool, sqlx::query(&peek).bind(BATCH).fetch_all(pool)).await;
    assert_eq!(rows.expect("old peek").len(), ids.len());
    let recheck =
        format!("SELECT 1 FROM search_documents WHERE id = ?1 AND {VEC0_ANTI_JOIN} LIMIT 1");
    let mut recheck_pages = 0;
    for id in ids {
        let (_, pages) =
            cold_pages(pool, sqlx::query(&recheck).bind(id).fetch_optional(pool)).await;
        recheck_pages += pages;
    }
    let count = format!("SELECT COUNT(*) FROM search_documents WHERE {VEC0_ANTI_JOIN}");
    let (_, count_pages) = cold_pages(pool, sqlx::query(&count).fetch_one(pool)).await;
    (peek_pages, recheck_pages, count_pages)
}

/// Fast: on an empty, unanalyzed database (the shipped state — the app never runs
/// ANALYZE) the peek is driven from the backlog's index with no sort, so `LIMIT`
/// stops it, and it never scans `search_documents` or touches vec0.
#[test]
fn the_backlog_peek_walks_its_index_newest_first() {
    run_async_test(async {
        let dir = test_dir("semantic-backfill-plan");
        std::fs::create_dir_all(&dir).expect("dir");
        let pool = connect_single_connection_owner_pool(&dir.join("app.sqlite3"), "plan-key")
            .await
            .expect("encrypted pool should open");
        let plan = explain(&pool, ANCHORS_MISSING_VECTOR_SQL).await;
        assert!(
            plan.starts_with(
                "SCAN search_document_vector_backlog USING COVERING INDEX \
                 search_document_vector_backlog_recent_idx"
            ) && plan.contains("SEARCH search_documents USING INTEGER PRIMARY KEY")
                && !plan.contains("TEMP B-TREE")
                && !plan.contains("search_document_vectors"),
            "peek must walk the backlog index newest-first and stop at LIMIT; plan was: {plan}"
        );
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    });
}

/// Pages one worker pass may read on the ~1.4 GB database: a few hundred for the
/// handful of B-tree descents a 6-anchor pass makes plus vec0's own insert,
/// which walks its 768 KB chunk blob to the write offset (<= ~200 pages).
const MAX_PASS_PAGES: i64 = 512;

#[test]
#[ignore = "seeds a ~1.4 GB database; run with --release --ignored"]
fn one_backfill_pass_reads_a_few_pages_not_the_vector_table() {
    run_async_test(async {
        let dir = test_dir("semantic-backfill-scale");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("app.sqlite3");
        let pool = connect_single_connection_owner_pool(&path, "scale-check-key")
            .await
            .expect("encrypted pool should open");
        let store = SemanticSearchStore::new(CaptureDb::single(pool.clone()));
        seed(&pool, &store).await;
        println!(
            "seeded {DOCUMENTS} documents, {:.0} MB on disk",
            std::fs::metadata(&path).expect("db file").len() as f64 / 1e6
        );

        // The app never runs ANALYZE, so the unanalyzed plan is the shipped one.
        // Measure the peek both ways in case statistics change the plan.
        let (_, unanalyzed_peek_pages) =
            cold_pages(&pool, store.anchors_missing_vector(BATCH)).await;
        sqlx::query("ANALYZE")
            .execute(&pool)
            .await
            .expect("analyze");
        println!(
            "peek plan: {}",
            explain(&pool, ANCHORS_MISSING_VECTOR_SQL).await
        );

        let (peek, peek_pages) = cold_pages(&pool, store.anchors_missing_vector(BATCH)).await;
        let peek = peek.expect("peek");
        let ids: Vec<i64> = peek.iter().map(|anchor| anchor.anchor_id).collect();
        let newest_first: Vec<i64> = (0..BACKLOG).map(|n| DOCUMENTS - 1 - 2 * n).collect();
        assert_eq!(
            ids, newest_first,
            "the peek drains the newest anchors first"
        );

        let (old_peek, old_recheck, old_count) = vec0_anti_join_pass_pages(&pool, &ids).await;

        let mut recheck_pages = 0;
        for id in &ids {
            let (still, pages) = cold_pages(&pool, store.anchor_still_missing_vector(*id)).await;
            assert!(still.expect("recheck"));
            recheck_pages += pages;
        }

        let unit = vec![1.0 / (DIMENSION as f32).sqrt(); DIMENSION];
        let pairs: Vec<_> = ids.iter().map(|id| (*id, unit.clone())).collect();
        let (stored, store_pages) = cold_pages(
            &pool,
            store.store_vectors_if_model_matches(MODEL_ID, &pairs),
        )
        .await;
        assert!(stored.expect("store").iter().all(|stored| *stored));

        let (remaining, count_pages) =
            cold_pages(&pool, store.count_anchors_missing_vector()).await;
        assert_eq!(remaining.expect("count"), 0);
        // Resume: a stored anchor leaves the work list, so the next pass idles.
        assert!(store
            .anchors_missing_vector(BATCH)
            .await
            .expect("peek")
            .is_empty());

        let mb = |pages: i64| pages as f64 * 4096.0 / 1e6;
        let old_pass = old_peek + old_recheck + store_pages;
        let pass = peek_pages + recheck_pages + store_pages;
        println!(
            "cold pages (4 KiB), {BACKLOG}-anchor pass:\n  \
             vec0 anti-join: peek {old_peek}, re-check x{BACKLOG} {old_recheck}, \
             store {store_pages}, count {old_count} -> pass {old_pass} ({:.1} MB)\n  \
             backlog:        peek {peek_pages} (unanalyzed {unanalyzed_peek_pages}), \
             re-check x{BACKLOG} {recheck_pages}, store {store_pages}, count {count_pages} \
             -> pass {pass} ({:.2} MB), {:.0}x fewer",
            mb(old_pass),
            mb(pass),
            old_pass as f64 / pass as f64,
        );
        assert!(
            pass <= MAX_PASS_PAGES && unanalyzed_peek_pages <= MAX_PASS_PAGES,
            "a {BACKLOG}-anchor backfill pass read {pass} pages ({:.1} MB) of a ~1.4 GB index; \
             something is scanning again",
            mb(pass)
        );

        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    });
}
