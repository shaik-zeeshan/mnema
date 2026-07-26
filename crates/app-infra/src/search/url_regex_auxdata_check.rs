//! Falsification check for the `db.rs` claim that sqlx's `regexp` UDF "caches the
//! compiled `Regex` in SQLite auxdata so a scan compiles it once" when the URL
//! refinement passes the pattern as a BOUND PARAMETER (`push_bind`), not a literal.
//!
//! ```text
//! cargo test -p app-infra --release --lib url_regex_auxdata -- --ignored --nocapture
//! ```
//!
//! Method: `regex::Regex::new` is ~4 orders of magnitude more expensive for a
//! large alternation than `is_match` is on a 30-char haystack. Run the same
//! `REGEXP` predicate over N rows with a compile-heavy pattern and with a
//! compile-cheap pattern that matches the same rows. If the compiled regex is
//! cached per statement, the two cost about the same; if it is recompiled per
//! scanned row, the heavy pattern costs N × compile.

use std::time::Instant;

use super::test_support::{run_async_test, test_dir};
use crate::AppInfra;

const ROWS: i64 = 20_000;

/// ~4 KB alternation: cheap to match (fails on the first byte for every row),
/// expensive to compile.
fn compile_heavy_pattern() -> String {
    let alternatives = (0..400)
        .map(|n| format!("zzq{n}[a-f0-9]{{4}}"))
        .collect::<Vec<_>>()
        .join("|");
    format!("^(?:{alternatives})$")
}

#[test]
#[ignore = "timing check; run with --release --ignored"]
fn url_regexp_compiles_the_bound_pattern_once_per_query() {
    run_async_test(async {
        let dir = test_dir("url-regex-auxdata");
        let infra = AppInfra::initialize(&dir)
            .await
            .expect("infra should initialize");
        let pool = infra.pool();
        sqlx::query(
            "WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?1) \
             INSERT INTO frame_metadata_snapshots (id, normalized_hash, snapshot_json) \
             SELECT n, 'hash-' || n, 'github.com/mnema/mnema/pull/' || n FROM seq",
        )
        .bind(ROWS)
        .execute(pool)
        .await
        .expect("rows should insert");

        let heavy = compile_heavy_pattern();
        let cheap = "^zzqnothingmatchesthis$".to_string();

        // One `Regex::new` of each pattern, for scale.
        let started = Instant::now();
        let _ = regex::Regex::new(&heavy).expect("heavy pattern compiles");
        let heavy_compile = started.elapsed();
        let started = Instant::now();
        let _ = regex::Regex::new(&cheap).expect("cheap pattern compiles");
        let cheap_compile = started.elapsed();

        let scan = |pattern: String| {
            let infra = &infra;
            async move {
                let started = Instant::now();
                let matched: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM frame_metadata_snapshots \
                     WHERE COALESCE(snapshot_json, '') REGEXP ?1",
                )
                .bind(pattern)
                .fetch_one(infra.pool())
                .await
                .expect("regexp scan should run");
                assert_eq!(matched, 0, "neither pattern should match any seeded row");
                started.elapsed()
            }
        };

        // Warm the page cache with a scan that does no regex work at all.
        let _ = scan(cheap.clone()).await;
        let cheap_scan = scan(cheap.clone()).await;
        let heavy_scan = scan(heavy.clone()).await;

        println!(
            "compile: heavy {heavy_compile:?} / cheap {cheap_compile:?}\n\
             {ROWS}-row bound-parameter REGEXP scan: heavy {heavy_scan:?} / cheap {cheap_scan:?}\n\
             heavy overhead vs cheap: {:?} ({:.1} compiles' worth)",
            heavy_scan.saturating_sub(cheap_scan),
            heavy_scan.saturating_sub(cheap_scan).as_secs_f64() / heavy_compile.as_secs_f64()
        );

        // If auxdata caches the bound pattern, the heavy scan pays ONE compile.
        // Allow a generous 5 compiles of slack before calling it per-row.
        assert!(
            heavy_scan.saturating_sub(cheap_scan) < heavy_compile * 5,
            "a bound-parameter REGEXP over {ROWS} rows paid \
             {:?} of extra pattern-compilation — the compiled Regex is NOT cached per statement",
            heavy_scan.saturating_sub(cheap_scan)
        );
    });
}
