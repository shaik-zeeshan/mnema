//! Fast plan checks for the statements the 2026-10-17 `stmt-cost` traces
//! caught re-reading app.sqlite3 (41 of 44.6 GB of page reads in 31 minutes,
//! then the user-context capture window at ~150 MB per distillation window).
//!
//! The app never runs `ANALYZE`, so the plan an empty migrated database gets is
//! the plan every install gets: each check asserts the statement seeks instead
//! of materializing or scanning a table that grows with capture history.

use sqlx::Row;

use super::equivalent_reuse::CANONICAL_DOCUMENT_FOR_RELATED_FRAME_SQL;
use super::test_support::{run_async_test, test_dir};
use crate::capture_retention::FINALIZED_SCREEN_SEGMENTS_IN_WINDOW_SQL;
use crate::processing::WORKSPACE_OCR_JOBS_SQL;
use crate::usage_charts::capture_hours::CAPTURE_HOURS_SQL;
use crate::usage_charts::USAGE_WINDOW_FRAMES_SQL;
use crate::user_context::subject_vectors::SUBJECTS_NEEDING_EMBEDDING_SQL;
use crate::user_context::capture_source::{
    AUDIO_SEGMENTS_IN_WINDOW_WITH_LATEST_TRANSCRIPTION_SQL, FRAMES_IN_WINDOW_WITH_LATEST_OCR_SQL,
};
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
            !segments.contains("SCAN frames")
                && segments.contains("frames_captured_at_idx (captured_at>? AND captured_at<?)"),
            "the window's frames sub-select must range-scan captured_at from both ends; plan was: {segments}"
        );
        // ...and the outer loop must probe only the matched segments by id, never
        // walk every screen segment ever recorded (`source_kind=?`).
        assert!(
            segments.starts_with("SEARCH capture_segments USING INTEGER PRIMARY KEY")
                && segments.contains("capture_segments_ended_at_idx (ended_at>? AND ended_at<?)"),
            "the window's segments must come from range scans, not every screen segment; plan was: {segments}"
        );

        // Was driven from every frame OCR job: ~86 MB per workspace check.
        let workspace = plan(&infra, WORKSPACE_OCR_JOBS_SQL).await;
        assert!(
            workspace.starts_with("SEARCH frames USING")
                && workspace.contains("frames_file_path_idx")
                && workspace.contains("(subject_type=? AND subject_id=?)"),
            "workspace OCR lookup must range-scan frames by path first; plan was: {workspace}"
        );

        // Was a GROUP BY over every OCR result: ~150 MB per distillation window.
        let capture_window = plan(&infra, FRAMES_IN_WINDOW_WITH_LATEST_OCR_SQL).await;
        assert!(
            capture_window.starts_with("SEARCH frames USING")
                && capture_window.contains("(captured_at>? AND captured_at<?)")
                && !capture_window.contains("SCAN processing_results"),
            "capture window must range-scan frames, then probe each one's results; plan was: {capture_window}"
        );

        // The audio half: must range-scan started_at from both ends, not walk
        // every audio segment recorded before the window's end.
        let audio_window = plan(
            &infra,
            AUDIO_SEGMENTS_IN_WINDOW_WITH_LATEST_TRANSCRIPTION_SQL,
        )
        .await;
        assert!(
            audio_window.starts_with("SEARCH audio_segments USING")
                && audio_window.contains("(started_at>? AND started_at<?)")
                && !audio_window.contains("SCAN processing_results"),
            "capture window audio must range-scan started_at from both ends; plan was: {audio_window}"
        );

        // Fetched each frame's whole row: ~30 MB per Insights refresh.
        let usage = plan(&infra, USAGE_WINDOW_FRAMES_SQL).await;
        assert!(
            usage.contains("SEARCH frames USING COVERING INDEX frames_usage_window_idx"),
            "usage charts must read frames from the covering index only; plan was: {usage}"
        );

        // The date picker's density: one walk of the same covering index.
        let hours = plan(&infra, CAPTURE_HOURS_SQL).await;
        assert!(
            hours.contains("SEARCH frames USING COVERING INDEX frames_usage_window_idx")
                && !hours.contains("SCAN frames"),
            "capture hours must read frames from the covering index only; plan was: {hours}"
        );

        // Read every Conclusion row and walked each vector's BLOB: ~12 MB per 30 s poll.
        let subjects = plan(&infra, SUBJECTS_NEEDING_EMBEDDING_SQL).await;
        assert!(
            subjects.contains("USING COVERING INDEX user_context_conclusions_subject_status_idx")
                && subjects.contains("USING COVERING INDEX user_context_subject_vectors_embedded_idx"),
            "subject poll must read both tables from indexes only; plan was: {subjects}"
        );

        drop(infra);
        let _ = std::fs::remove_dir_all(&dir);
    });
}
