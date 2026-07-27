//! Scale checks for the startup URL backfill added by migration 0050.
//!
//! These are `#[ignore]`d because they seed hundreds of thousands of rows; run
//! them explicitly:
//!
//! ```text
//! cargo test -p app-infra --release --lib url_backfill -- --ignored --nocapture
//! ```

use std::time::Instant;

use sqlx::Row;

use super::projection::{
    backfill_missing_app_bundle_id_projection, backfill_missing_url_projection,
};
use super::test_support::{run_async_test, test_dir};
use crate::db::CaptureDb;
use crate::AppInfra;

/// Rows in the seeded `search_documents` table. A continuous screen recorder
/// accumulates millions; 300k keeps the check runnable in ~a minute while still
/// being large enough for the scan-vs-seek difference to be unambiguous.
const SEEDED_DOCUMENTS: i64 = 300_000;

async fn seed(infra: &AppInfra, rows: i64, url_value: &str) {
    // One shared metadata snapshot row, ~340 bytes of JSON — the realistic shape
    // for a browser frame (bundle id + app name + a long window title + a URL).
    let snapshot_json = serde_json::to_string(&capture_metadata::FrameMetadataSnapshot {
        app_bundle_id: Some("com.apple.Safari".to_string()),
        app_name: Some("Safari".to_string()),
        window_title: Some(
            "Pull requests · mnema/mnema · feat(broker): filter brokered search and timeline by \
             captured URL — Safari"
                .to_string(),
        ),
        window_id: Some(42),
        browser_url: Some(
            "https://github.com/mnema/mnema/pulls?q=is%3Apr+is%3Aopen+sort%3Aupdated-desc"
                .to_string(),
        ),
        display_id: Some(1),
        metadata_redaction_reason: None,
        metadata_redaction_source_id: None,
    })
    .expect("snapshot should serialize");

    let pool = infra.pool();
    sqlx::query(
        "INSERT INTO frame_metadata_snapshots (id, normalized_hash, snapshot_json) \
         VALUES (1, 'seed-hash', ?1)",
    )
    .bind(&snapshot_json)
    .execute(pool)
    .await
    .expect("snapshot should insert");

    // Seed entirely inside SQLite so the harness measures the backfill, not the
    // seeding. `body_text` is padded to ~2 KB, the realistic size of one frame's
    // OCR text — that is what a full table scan of `search_documents` has to
    // walk past.
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(pool)
        .await
        .ok();
    let mut transaction = pool.begin().await.expect("tx should begin");
    sqlx::query(
        "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?1) \
         INSERT INTO frames (id, session_id, file_path, captured_at, metadata_snapshot_id) \
         SELECT n, 'seed-session', '/tmp/seed-' || n || '.jpg', '2026-05-17T10:00:00Z', 1 FROM seq",
    )
    .bind(rows)
    .execute(&mut *transaction)
    .await
    .expect("frames should insert");
    sqlx::query(
        "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?1) \
         INSERT INTO search_documents (\
             id, anchor_type, frame_id, absolute_start_at, absolute_end_at, session_id, \
             app_bundle_id, app_name, app_name_search_key, window_title, url, group_key, \
             text_source_kind, body_text, context_text) \
         SELECT n, 'frame', n, '2026-05-17T10:00:00Z', '2026-05-17T10:00:01Z', 'seed-session', \
                'com.apple.safari', 'Safari', 'safari', 'Pull requests', ?2, 'frame:' || n, \
                'direct', 'seed body ' || n || ' ' || hex(zeroblob(1000)), 'Safari' FROM seq",
    )
    .bind(rows)
    .bind(url_value)
    .execute(&mut *transaction)
    .await
    .expect("documents should insert");
    transaction.commit().await.expect("tx should commit");
    sqlx::query("ANALYZE").execute(pool).await.ok();
}

async fn explain(infra: &AppInfra, sql: &str) -> String {
    sqlx::query(&format!("EXPLAIN QUERY PLAN {sql}"))
        .fetch_all(infra.pool())
        .await
        .expect("explain should run")
        .into_iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// RED: the *steady-state* cost of the new startup backfill — every launch after
/// the one that drained it, forever, on every install.
///
/// The PR calls this backfill the "same shape" as the app-identity backfills and
/// justifies having no index with "LIKE and REGEXP are both unsargable". Both
/// claims miss that the backfill's own `WHERE ... url IS NULL` probe is an
/// *equality-shaped* predicate that runs on every startup. `0017` gave the
/// bundle-id sibling an index whose leading columns are exactly its probe, so
/// that sibling costs an empty index seek once drained. The url probe has
/// nothing to seek, so it re-scans the whole `search_documents` table (2 KB
/// `body_text` per row) at every launch.
#[test]
#[ignore = "seeds 300k rows; run with --release --ignored"]
fn drained_url_backfill_rescans_the_whole_table_every_startup() {
    run_async_test(async {
        let dir = test_dir("url-backfill-drained");
        let infra = AppInfra::initialize(&dir)
            .await
            .expect("infra should initialize");
        // Steady state: every row already projected (the insert path binds
        // `unwrap_or_default()`, so a live row's url is '' and never NULL).
        seed(&infra, SEEDED_DOCUMENTS, "").await;

        // Verbatim from `backfill_missing_url_projection`.
        let url_plan = explain(
            &infra,
            "SELECT search_documents.id, frame_metadata_snapshots.snapshot_json \
             FROM search_documents \
             LEFT JOIN frames ON frames.id = search_documents.frame_id \
             LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
             WHERE search_documents.url IS NULL \
             LIMIT 2000",
        )
        .await;
        let bundle_plan = explain(
            &infra,
            "SELECT search_documents.id, frame_metadata_snapshots.snapshot_json \
             FROM search_documents \
             JOIN frames ON frames.id = search_documents.frame_id \
             LEFT JOIN frame_metadata_snapshots ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
             WHERE search_documents.anchor_type = 'frame' AND search_documents.app_bundle_id IS NULL",
        )
        .await;
        println!("url    plan: {url_plan}");
        println!("bundle plan: {bundle_plan}");

        let db = CaptureDb::single(infra.pool().clone());
        let db = &db;
        let started = Instant::now();
        backfill_missing_url_projection(db)
            .await
            .expect("url backfill should run");
        let url_elapsed = started.elapsed();
        let started = Instant::now();
        backfill_missing_app_bundle_id_projection(db)
            .await
            .expect("bundle backfill should run");
        let bundle_elapsed = started.elapsed();
        println!(
            "drained backfill over {SEEDED_DOCUMENTS} docs — url: {url_elapsed:?}, \
             app_bundle_id (sibling with an index): {bundle_elapsed:?}"
        );

        assert!(
            url_plan.contains("search_documents_url_backfill_idx"),
            "a drained url backfill must seek the un-projected rows, not visit every frame \
             document, on every startup; plan was: {url_plan}"
        );
        assert!(
            url_elapsed < bundle_elapsed * 50,
            "a drained url backfill must cost the same order as the indexed sibling it claims to \
             mirror; url {url_elapsed:?} vs app_bundle_id {bundle_elapsed:?}"
        );
        drop(infra);
        let _ = std::fs::remove_dir_all(&dir);
    });
}

/// RED: peak memory of the *first* launch after upgrade. `fetch_all` with no
/// LIMIT materializes one `SqliteRow` per un-projected frame document — each
/// carrying a full `snapshot_json` copy — and the `updates` Vec is built while
/// that Vec is still alive.
#[test]
#[ignore = "seeds 300k rows; run with --release --ignored"]
fn first_launch_url_backfill_materializes_the_whole_table() {
    run_async_test(async {
        let dir = test_dir("url-backfill-cold");
        let infra = AppInfra::initialize(&dir)
            .await
            .expect("infra should initialize");
        // First launch after upgrade: every pre-0050 row has url IS NULL.
        seed(&infra, SEEDED_DOCUMENTS, "pending").await;
        sqlx::query("UPDATE search_documents SET url = NULL")
            .execute(infra.pool())
            .await
            .expect("null out url");

        // One-time upgrade cost of the partial index itself: at migration time
        // every existing row satisfies `url IS NULL`, so this build is O(table).
        let started = Instant::now();
        sqlx::query(
            "CREATE INDEX search_documents_url_backfill_idx_probe \
             ON search_documents (id) WHERE url IS NULL",
        )
        .execute(infra.pool())
        .await
        .expect("index should build");
        println!(
            "partial-index build over {SEEDED_DOCUMENTS} all-NULL rows (i.e. inside the \
             synchronous migration): {:?}",
            started.elapsed()
        );
        sqlx::query("DROP INDEX search_documents_url_backfill_idx_probe")
            .execute(infra.pool())
            .await
            .expect("probe index should drop");

        let before = rusage_maxrss();
        let started = Instant::now();
        backfill_missing_url_projection(&CaptureDb::single(infra.pool().clone()))
            .await
            .expect("url backfill should run");
        let elapsed = started.elapsed();
        let peak = rusage_maxrss();
        let growth = peak.saturating_sub(before);
        println!(
            "cold backfill over {SEEDED_DOCUMENTS} docs — {elapsed:?}, peak rss before {:.0} MB, \
             after {:.0} MB, growth {:.0} MB ({:.0} bytes/document)",
            before as f64 / 1e6,
            peak as f64 / 1e6,
            growth as f64 / 1e6,
            growth as f64 / SEEDED_DOCUMENTS as f64
        );

        // Same index, built once the backfill has drained every row (the deferred
        // placement): the scan is identical but there is nothing to insert.
        let started = Instant::now();
        sqlx::query(
            "CREATE INDEX search_documents_url_backfill_idx_probe2 \
             ON search_documents (id) WHERE url IS NULL",
        )
        .execute(infra.pool())
        .await
        .expect("index should build");
        println!(
            "partial-index build over {SEEDED_DOCUMENTS} already-drained rows: {:?}",
            started.elapsed()
        );

        // A bounded backfill holds one chunk at a time, so its peak does not scale
        // with the table. 300k documents costing ~200 MB extrapolates to ~3.3 GB at
        // the 5M-document installs this backfill actually ships to.
        assert!(
            growth < 100_000_000,
            "url backfill grew peak RSS by {growth} bytes over {SEEDED_DOCUMENTS} rows — \
             unbounded materialization does not survive a multi-million-row install"
        );
        drop(infra);
        let _ = std::fs::remove_dir_all(&dir);
    });
}

/// `ru_maxrss` is the process-wide peak resident set (bytes on Darwin).
fn rusage_maxrss() -> u64 {
    #[repr(C)]
    #[derive(Default)]
    struct Rusage {
        ru_utime: [i64; 2],
        ru_stime: [i64; 2],
        ru_maxrss: i64,
        rest: [i64; 14],
    }
    extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    let mut usage = Rusage::default();
    unsafe {
        if getrusage(0, &mut usage) != 0 {
            return 0;
        }
    }
    usage.ru_maxrss as u64
}
