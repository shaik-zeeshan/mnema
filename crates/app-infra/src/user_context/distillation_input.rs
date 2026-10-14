//! **Incremental Conclusion-distillation input.** A pass sends only the
//! Activities inserted or extended since the last successful pass, plus a few
//! recent already-distilled ones for context. The position is a durable
//! watermark in `app_settings`, so a restart neither re-sends old Activities nor
//! skips new ones, and a quiet stretch costs no tokens at all.
//!
//! An Activity's change stamp is `MAX(created_at_ms, updated_at_ms)` (migration
//! `0058`; continuation stamps `updated_at_ms`). Changed rows are read oldest
//! change first, so a History Backfill that adds hundreds of Activities is
//! distilled in bounded chunks across passes instead of only its newest slice.

use capture_types::Activity;
use sqlx::Row;

use super::store::{map_activity, UserContextStore};
use crate::Result;

/// `app_settings` key holding the watermark as `"<changed_at_ms>:<activity_id>"`.
const WATERMARK_KEY: &str = "user_context.distillation_watermark";

const ACTIVITY_COLUMNS: &str = "id, title, summary, category, focus, corrected_category, \
    category_corrected, corrected_focus, focus_corrected, started_at_ms, ended_at_ms, \
    created_at_ms, MAX(created_at_ms, COALESCE(updated_at_ms, 0)) AS changed_at_ms";

// ponytail: full scan over user_context_activities (thousands of rows, once per
// pass); add an index on the change stamp if the table ever reaches 100k+.
const CHANGED_AFTER: &str = "(MAX(created_at_ms, COALESCE(updated_at_ms, 0)), id) > (?1, ?2)";

/// The last Activity a successful distillation pass covered, in change order.
/// `(changed_at_ms, activity_id)` rather than a bare timestamp, so rows sharing
/// a millisecond are neither re-sent nor skipped across a chunk boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DistillationWatermark {
    pub changed_at_ms: i64,
    pub activity_id: i64,
}

/// What one distillation pass should send.
#[derive(Debug, Default)]
pub struct DistillationInput {
    /// Activities inserted or extended since the watermark, oldest change first.
    pub changed: Vec<Activity>,
    /// The most recent Activities not in `changed`, newest first — context only.
    pub context: Vec<Activity>,
    /// Where the watermark moves once this pass succeeds; `None` = nothing changed.
    pub next_watermark: Option<DistillationWatermark>,
}

impl UserContextStore {
    /// The stored watermark, or `None` before the first successful pass (or if
    /// the value is unreadable — the next pass then re-seeds it).
    pub async fn distillation_watermark(&self) -> Result<Option<DistillationWatermark>> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?1")
                .bind(WATERMARK_KEY)
                .fetch_optional(self.db.read())
                .await?;
        Ok(value.and_then(|value| {
            let (ms, id) = value.trim().split_once(':')?;
            Some(DistillationWatermark {
                changed_at_ms: ms.parse().ok()?,
                activity_id: id.parse().ok()?,
            })
        }))
    }

    pub async fn set_distillation_watermark(&self, watermark: DistillationWatermark) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(WATERMARK_KEY)
        .bind(format!(
            "{}:{}",
            watermark.changed_at_ms, watermark.activity_id
        ))
        .execute(self.db.write())
        .await?;
        Ok(())
    }

    /// Whether any Activity was inserted or extended since the last successful
    /// distillation pass — the worker's "is a pass worth its tokens?" check.
    pub async fn activities_changed_since_distillation(&self) -> Result<bool> {
        let Some(mark) = self.distillation_watermark().await? else {
            let any: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_context_activities)")
                    .fetch_one(self.db.read())
                    .await?;
            return Ok(any);
        };
        let sql =
            format!("SELECT EXISTS(SELECT 1 FROM user_context_activities WHERE {CHANGED_AFTER})");
        Ok(sqlx::query_scalar(&sql)
            .bind(mark.changed_at_ms)
            .bind(mark.activity_id)
            .fetch_one(self.db.read())
            .await?)
    }

    /// The next pass's input: up to `changed_limit` Activities changed since the
    /// watermark (oldest change first) plus up to `context_limit` recent ones
    /// outside that set. With no watermark yet (first pass ever), "changed" is
    /// the newest `changed_limit` by change stamp — older history is not
    /// re-billed on upgrade; every later insert/extend is picked up.
    pub async fn distillation_input(
        &self,
        changed_limit: i64,
        context_limit: i64,
    ) -> Result<DistillationInput> {
        let mark = self.distillation_watermark().await?;
        let rows = match mark {
            Some(mark) => {
                let sql = format!(
                    "SELECT {ACTIVITY_COLUMNS} FROM user_context_activities WHERE {CHANGED_AFTER} \
                     ORDER BY changed_at_ms ASC, id ASC LIMIT ?3"
                );
                sqlx::query(&sql)
                    .bind(mark.changed_at_ms)
                    .bind(mark.activity_id)
                    .bind(changed_limit)
                    .fetch_all(self.db.read())
                    .await?
            }
            None => {
                let sql = format!(
                    "SELECT {ACTIVITY_COLUMNS} FROM user_context_activities \
                     ORDER BY changed_at_ms DESC, id DESC LIMIT ?1"
                );
                let mut rows = sqlx::query(&sql)
                    .bind(changed_limit)
                    .fetch_all(self.db.read())
                    .await?;
                rows.reverse();
                rows
            }
        };

        let mut input = DistillationInput::default();
        for row in rows {
            let changed_at_ms: i64 = row.get("changed_at_ms");
            let mut activity = map_activity(row);
            activity.evidence = self.list_activity_evidence(activity.id).await?;
            input.next_watermark = Some(DistillationWatermark {
                changed_at_ms,
                activity_id: activity.id,
            });
            input.changed.push(activity);
        }
        if input.changed.is_empty() {
            return Ok(input);
        }

        // Newest `changed + context` by start time minus the changed rows leaves
        // at least `context_limit` others whenever that many exist.
        let changed_ids: std::collections::HashSet<i64> =
            input.changed.iter().map(|activity| activity.id).collect();
        input.context = self
            .activities_for_distillation(input.changed.len() as i64 + context_limit)
            .await?
            .into_iter()
            .filter(|activity| !changed_ids.contains(&activity.id))
            .take(context_limit as usize)
            .collect();
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::CaptureDb;
    use crate::user_context::{NewActivity, NewActivityEvidence};
    use sqlx::sqlite::SqlitePoolOptions;

    const SCHEMA: &str = "
        CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
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

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    async fn store() -> UserContextStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory db");
        sqlx::raw_sql(SCHEMA).execute(&pool).await.expect("schema");
        UserContextStore::new(CaptureDb::single(pool))
    }

    fn activity(started_at_ms: i64) -> NewActivity {
        NewActivity {
            title: format!("a{started_at_ms}"),
            summary: String::new(),
            category: None,
            focus: None,
            started_at_ms,
            ended_at_ms: started_at_ms + 10,
            derivation_run_id: None,
            evidence: vec![NewActivityEvidence {
                subject_type: "frame".to_string(),
                subject_id: started_at_ms,
                captured_at_ms: Some(started_at_ms),
                is_headline: true,
            }],
        }
    }

    fn ids(activities: &[Activity]) -> Vec<i64> {
        activities.iter().map(|activity| activity.id).collect()
    }

    /// Advance the watermark the way a successful pass does.
    async fn commit(store: &UserContextStore, input: &DistillationInput) {
        store
            .set_distillation_watermark(input.next_watermark.expect("something changed"))
            .await
            .expect("set watermark");
    }

    #[test]
    fn sends_only_changed_activities_plus_context_and_persists_the_watermark() {
        block_on(async {
            let store = store().await;
            let mut all = Vec::new();
            for start in 1..=8 {
                all.push(
                    store
                        .insert_activity_with_evidence(activity(start * 1_000))
                        .await
                        .unwrap(),
                );
            }

            // First pass ever: newest-by-change slice, no older history re-billed.
            let first = store.distillation_input(3, 2).await.unwrap();
            assert_eq!(ids(&first.changed), all[5..].to_vec());
            assert_eq!(ids(&first.context), vec![all[4], all[3]]);
            commit(&store, &first).await;

            // Nothing changed since: no pass input, and the check says so.
            assert!(!store.activities_changed_since_distillation().await.unwrap());
            assert!(store
                .distillation_input(3, 2)
                .await
                .unwrap()
                .next_watermark
                .is_none());

            // One insert and one continuation extend (of an OLD row) since the
            // watermark; the sleeps keep the millisecond change stamps distinct.
            let tick = || std::thread::sleep(std::time::Duration::from_millis(2));
            tick();
            let new_id = store
                .insert_activity_with_evidence(activity(9_000))
                .await
                .unwrap();
            tick();
            assert!(store
                .extend_activity_with_evidence(all[0], activity(1_500))
                .await
                .unwrap());
            assert!(store.activities_changed_since_distillation().await.unwrap());

            let next = store.distillation_input(3, 2).await.unwrap();
            assert_eq!(
                ids(&next.changed),
                vec![new_id, all[0]],
                "oldest change first"
            );
            assert_eq!(
                ids(&next.context),
                vec![all[7], all[6]],
                "newest unchanged rows"
            );
            commit(&store, &next).await;

            // The watermark lives in the DB, so a fresh store handle (restart) sees it.
            let reopened = UserContextStore::new(store.db.clone());
            assert!(!reopened
                .activities_changed_since_distillation()
                .await
                .unwrap());
        });
    }

    #[test]
    fn a_large_backlog_is_drained_in_bounded_chunks() {
        block_on(async {
            let store = store().await;
            let first = store
                .insert_activity_with_evidence(activity(100_000))
                .await
                .unwrap();
            store
                .set_distillation_watermark(DistillationWatermark {
                    changed_at_ms: 0,
                    activity_id: first - 1,
                })
                .await
                .unwrap();
            // A backfill inserts older history after the watermark.
            let mut backfill = vec![first];
            for start in 1..=6 {
                backfill.push(
                    store
                        .insert_activity_with_evidence(activity(start))
                        .await
                        .unwrap(),
                );
            }

            let mut seen = Vec::new();
            loop {
                let input = store.distillation_input(3, 1).await.unwrap();
                if input.next_watermark.is_none() {
                    break;
                }
                assert!(input.changed.len() <= 3);
                seen.extend(ids(&input.changed));
                commit(&store, &input).await;
            }
            assert_eq!(
                seen, backfill,
                "every row sent exactly once, in change order"
            );
        });
    }
}
