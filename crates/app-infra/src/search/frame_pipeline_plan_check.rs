//! Fast plan checks for the three statements the 2026-10-17 `stmt-cost` trace
//! caught re-reading app.sqlite3 (41 of 44.6 GB of page reads in 31 minutes).
//!
//! The app never runs `ANALYZE`, so the plan an empty migrated database gets is
//! the plan every install gets: each check asserts the statement seeks instead
//! of materializing or scanning a table that grows with capture history.

use sqlx::Row;

use super::equivalent_reuse::CANONICAL_DOCUMENT_FOR_RELATED_FRAME_SQL;
use super::test_support::{run_async_test, test_dir};
use crate::capture_retention::FINALIZED_SCREEN_SEGMENTS_IN_WINDOW_SQL;
use crate::processing::WORKSPACE_OCR_JOBS_SQL;
use crate::AppInfra;

async fn plan(infra: &AppInfra, sql: &str) -> String {
    sqlx::query(&format!("EXPLAIN QUERY PLAN {sql}"))
        .bind("a")
        .bind("b")
        .bind("c")
        .bind("d")
        .fetch_all(infra.pool())
        .await
        .expect("explain should run")
        .into_iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[test]
fn frame_pipeline_hot_statements_seek_instead_of_scanning() {
    run_async_test(async {
        let dir = test_dir("frame-pipeline-plans");
        let infra = AppInfra::initialize(&dir)
            .await
            .expect("infra should initialize");

        // Was `LIST SUBQUERY` over every OCR result: ~93 MB per equivalent frame.
        let canonical = plan(&infra, CANONICAL_DOCUMENT_FOR_RELATED_FRAME_SQL).await;
        assert!(
            !canonical.contains("LIST SUBQUERY")
                && canonical.contains("SEARCH processing_results USING INTEGER PRIMARY KEY"),
            "canonical lookup must probe processing_results by id; plan was: {canonical}"
        );

        // Was `SCAN frames` per scrub-preview call: ~72 MB each.
        let segments = plan(&infra, FINALIZED_SCREEN_SEGMENTS_IN_WINDOW_SQL).await;
        assert!(
            !segments.contains("SCAN frames") && segments.contains("frames_captured_at_idx"),
            "the window's frames sub-select must range-scan captured_at; plan was: {segments}"
        );

        // Was driven from every frame OCR job: ~86 MB per workspace check.
        let workspace = plan(&infra, WORKSPACE_OCR_JOBS_SQL).await;
        assert!(
            workspace.starts_with("SEARCH frames USING")
                && workspace.contains("frames_file_path_idx")
                && workspace.contains("(subject_type=? AND subject_id=?)"),
            "workspace OCR lookup must range-scan frames by path first; plan was: {workspace}"
        );

        drop(infra);
        let _ = std::fs::remove_dir_all(&dir);
    });
}
