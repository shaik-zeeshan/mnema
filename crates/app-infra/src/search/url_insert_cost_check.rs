//! Does the migration-0050 partial backfill index cost anything on "the hottest
//! insert path in the schema"? The PR rejected any index on `url` because it
//! "would only ever be scanned while adding write cost on the hottest insert
//! path". A PARTIAL index over `url IS NULL` is the exception: the insert binds
//! `url` to `''` (never NULL), so a live row never satisfies the index predicate
//! and never produces an index entry.
//!
//! ```text
//! cargo test -p app-infra --release --lib url_insert_cost -- --ignored --nocapture
//! ```

use std::time::Instant;

use super::test_support::{run_async_test, test_dir};
use crate::AppInfra;

const INSERTS: i64 = 50_000;

async fn timed_bulk_insert(infra: &AppInfra, first_id: i64, rows: i64) -> std::time::Duration {
    let started = Instant::now();
    sqlx::query(
        "WITH RECURSIVE seq(n) AS (SELECT ?1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?2) \
         INSERT INTO search_documents (\
             id, anchor_type, frame_id, absolute_start_at, absolute_end_at, session_id, \
             app_bundle_id, app_name, app_name_search_key, window_title, url, group_key, \
             text_source_kind, body_text, context_text) \
         SELECT n, 'frame', n, '2026-05-17T10:00:00Z', '2026-05-17T10:00:01Z', 'seed-session', \
                'com.apple.safari', 'Safari', 'safari', 'Pull requests', '', 'frame:' || n, \
                'direct', 'seed body ' || n, 'Safari' FROM seq",
    )
    .bind(first_id)
    .bind(first_id + rows - 1)
    .execute(infra.pool())
    .await
    .expect("insert should succeed");
    started.elapsed()
}

#[test]
#[ignore = "timing check; run with --release --ignored"]
fn partial_backfill_index_is_free_on_the_search_document_insert_path() {
    run_async_test(async {
        let dir = test_dir("url-insert-cost");
        let infra = AppInfra::initialize(&dir)
            .await
            .expect("infra should initialize");
        // Each document needs its frame (CHECK + FK), so seed those first.
        sqlx::query(
            "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?1) \
             INSERT INTO frames (id, session_id, file_path, captured_at) \
             SELECT n, 'seed-session', '/tmp/seed-' || n || '.jpg', '2026-05-17T10:00:00Z' FROM seq",
        )
        .bind(INSERTS * 3)
        .execute(infra.pool())
        .await
        .expect("frames should insert");

        // Warm-up batch (index present, as shipped).
        let _ = timed_bulk_insert(&infra, 1, INSERTS).await;
        let with_index = timed_bulk_insert(&infra, INSERTS + 1, INSERTS).await;

        sqlx::query("DROP INDEX search_documents_url_backfill_idx")
            .execute(infra.pool())
            .await
            .expect("index should drop");
        let without_index = timed_bulk_insert(&infra, INSERTS * 2 + 1, INSERTS).await;

        println!(
            "{INSERTS} search_documents inserts — with the partial url index: {with_index:?}, \
             without it: {without_index:?} ({:+.1}%)",
            (with_index.as_secs_f64() / without_index.as_secs_f64() - 1.0) * 100.0
        );

        assert!(
            with_index.as_secs_f64() < without_index.as_secs_f64() * 1.15,
            "the partial backfill index must not measurably slow the insert path: \
             {with_index:?} with vs {without_index:?} without"
        );
        drop(infra);
        let _ = std::fs::remove_dir_all(&dir);
    });
}
