//! **Activity continuation**: a derivation window that picks up an Activity a
//! neighbouring window already wrote grows that row instead of inserting a
//! second one. Windows are only a few minutes long, so without this one
//! 8-minute video became three Activities with three different titles.
//!
//! The row keeps its `id`, so Conclusion evidence and the user's Category/Focus
//! corrections stay attached to it.

use super::store::{category_to_str, focus_to_str, NewActivity, UserContextStore};
use crate::Result;

impl UserContextStore {
    /// Grow Activity `activity_id` to also cover `draft`: its span widens to
    /// include the draft's, its title/summary become the draft's (the engine
    /// rewrote them for the whole Activity), a blank title/summary or a label
    /// the draft omits keeps the stored one, and the draft's evidence is appended (duplicates ignored).
    /// The existing headline stays: appended evidence is never the headline.
    ///
    /// Returns `false` when no such Activity exists (e.g. Delete Recent Capture
    /// removed it after the prompt was built); the caller inserts instead.
    pub async fn extend_activity_with_evidence(
        &self,
        activity_id: i64,
        draft: NewActivity,
    ) -> Result<bool> {
        let mut transaction = self.db.begin_write().await?;

        let updated = sqlx::query(
            "UPDATE user_context_activities SET \
                title = COALESCE(NULLIF(?2, ''), title), summary = COALESCE(NULLIF(?3, ''), summary), \
                category = COALESCE(?4, category), focus = COALESCE(?5, focus), \
                started_at_ms = MIN(started_at_ms, ?6), ended_at_ms = MAX(ended_at_ms, ?7), \
                updated_at_ms = ?8 \
             WHERE id = ?1",
        )
        .bind(activity_id)
        .bind(&draft.title)
        .bind(&draft.summary)
        .bind(draft.category.map(category_to_str))
        .bind(draft.focus.map(focus_to_str))
        .bind(draft.started_at_ms)
        .bind(draft.ended_at_ms)
        .bind(super::store::now_ms())
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if updated == 0 {
            return Ok(false);
        }

        for evidence in &draft.evidence {
            sqlx::query(
                "INSERT OR IGNORE INTO user_context_activity_evidence \
                    (activity_id, subject_type, subject_id, captured_at_ms, is_headline) \
                 VALUES (?1, ?2, ?3, ?4, 0)",
            )
            .bind(activity_id)
            .bind(&evidence.subject_type)
            .bind(evidence.subject_id)
            .bind(evidence.captured_at_ms)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::CaptureDb;
    use crate::user_context::NewActivityEvidence;
    use capture_types::ActivityCategory;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::Row;

    /// Only the columns the insert/extend/read paths touch.
    const SCHEMA: &str = "
        CREATE TABLE user_context_activities (id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL, summary TEXT NOT NULL, category TEXT, focus TEXT,
            started_at_ms INTEGER NOT NULL, ended_at_ms INTEGER NOT NULL,
            derivation_run_id INTEGER, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER,
            corrected_category TEXT, category_corrected INTEGER NOT NULL DEFAULT 0,
            corrected_focus TEXT, focus_corrected INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE user_context_activity_evidence (id INTEGER PRIMARY KEY AUTOINCREMENT,
            activity_id INTEGER NOT NULL, subject_type TEXT NOT NULL, subject_id INTEGER NOT NULL,
            captured_at_ms INTEGER, is_headline INTEGER NOT NULL DEFAULT 0,
            UNIQUE (activity_id, subject_type, subject_id));
    ";

    fn activity(title: &str, category: Option<ActivityCategory>, frames: &[i64]) -> NewActivity {
        NewActivity {
            title: title.to_string(),
            summary: format!("{title} summary"),
            category,
            focus: None,
            started_at_ms: frames[0],
            ended_at_ms: *frames.last().unwrap(),
            derivation_run_id: None,
            evidence: frames
                .iter()
                .map(|&id| NewActivityEvidence {
                    subject_type: "frame".to_string(),
                    subject_id: id,
                    captured_at_ms: Some(id),
                    is_headline: id == frames[0],
                })
                .collect(),
        }
    }

    #[test]
    fn extending_grows_the_same_row_and_keeps_its_headline_and_label() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let pool = SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect("sqlite::memory:")
                    .await
                    .expect("db");
                sqlx::raw_sql(SCHEMA).execute(&pool).await.expect("schema");
                let store = UserContextStore::new(CaptureDb::single(pool));

                let id = store
                    .insert_activity_with_evidence(activity(
                        "Watching a review",
                        Some(ActivityCategory::Entertainment),
                        &[1_000, 2_000],
                    ))
                    .await
                    .expect("insert");
                // Next window: frame 2_000 again (dup) + two new ones, no label.
                let grew = store
                    .extend_activity_with_evidence(
                        id,
                        activity("Watched the Gears of War PC review", None, &[2_000, 3_000, 4_000]),
                    )
                    .await
                    .expect("extend");
                assert!(grew);

                let rows = store.list_activities_in_range(0, 10_000).await.expect("list");
                assert_eq!(rows.len(), 1, "one Activity, not two");
                let row = &rows[0];
                assert_eq!(row.id, id);
                assert_eq!(row.title, "Watched the Gears of War PC review");
                assert_eq!((row.started_at_ms, row.ended_at_ms), (1_000, 4_000));
                assert_eq!(row.category, Some(ActivityCategory::Entertainment));

                let evidence = sqlx::query(
                    "SELECT subject_id, is_headline FROM user_context_activity_evidence \
                     WHERE activity_id = ?1 ORDER BY subject_id",
                )
                .bind(id)
                .fetch_all(store.db.read())
                .await
                .expect("evidence");
                let got: Vec<(i64, i64)> = evidence
                    .iter()
                    .map(|r| (r.get("subject_id"), r.get("is_headline")))
                    .collect();
                assert_eq!(got, vec![(1_000, 1), (2_000, 0), (3_000, 0), (4_000, 0)]);

                let missing = store
                    .extend_activity_with_evidence(999, activity("x", None, &[5_000]))
                    .await
                    .expect("extend missing");
                assert!(!missing, "a vanished row reports false so the caller inserts");
            });
    }
}
