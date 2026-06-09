//! `UserContextStore` — SQLite-backed storage for the User Context dossier.
//!
//! This slice (#93 foundation) owns only the `0022_*` tables:
//! `user_context_activities`, `user_context_activity_evidence`, and
//! `user_context_derivation_runs`. Conclusion / confidence / dismissal / wipe /
//! cascade methods land with their own migrations in later slices.
//!
//! Timestamps are INTEGER unix milliseconds set from Rust (see migration
//! `0022_user_context_activities.sql`); they are read/written as raw `i64`
//! columns with no RFC3339 parsing.

use sqlx::{sqlite::SqliteRow, Row, SqlitePool};
use time::OffsetDateTime;

use capture_types::{Activity, ActivityCategory, ActivityEvidenceRef, UserContextTokenUsage};

use crate::Result;

/// A new Activity (the evidence layer) plus the raw-capture evidence it is
/// grounded in, ready to persist via
/// [`UserContextStore::insert_activity_with_evidence`].
#[derive(Debug, Clone, PartialEq)]
pub struct NewActivity {
    pub title: String,
    pub summary: String,
    pub category: Option<ActivityCategory>,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub derivation_run_id: Option<i64>,
    pub evidence: Vec<NewActivityEvidence>,
}

/// One raw-capture evidence reference for a [`NewActivity`]. `subject_type` is
/// `"frame"` | `"audio_segment"` (mirrors `processing_jobs` subject types).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewActivityEvidence {
    pub subject_type: String,
    pub subject_id: i64,
    pub captured_at_ms: Option<i64>,
}

/// A new derivation-run ledger row. Records which window a derivation pass
/// covered (newest-first / skip-already-derived), its outcome, and its
/// (estimated) token usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDerivationRun {
    /// `'activity'` | `'conclusion'` | `'confidence'` | `'backfill'`.
    pub kind: String,
    pub window_start_ms: Option<i64>,
    pub window_end_ms: Option<i64>,
    /// `'running'` | `'completed'` | `'failed'` | `'skipped'`.
    pub status: String,
    pub activities_derived: i64,
    pub conclusions_derived: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub error: Option<String>,
}

/// SQLite-backed storage for the User Context dossier (Activities + evidence +
/// derivation runs in this slice).
#[derive(Clone)]
pub struct UserContextStore {
    pool: SqlitePool,
}

impl UserContextStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // --- #93: Activities + evidence ---------------------------------------

    /// Inserts an Activity and its evidence rows in a single transaction,
    /// returning the new Activity id. Duplicate evidence (same
    /// `activity_id`/`subject_type`/`subject_id`) is ignored.
    pub async fn insert_activity_with_evidence(&self, draft: NewActivity) -> Result<i64> {
        let created_at_ms = now_ms();
        let mut transaction = self.pool.begin().await?;

        let activity_id = sqlx::query(
            "INSERT INTO user_context_activities \
                (title, summary, category, started_at_ms, ended_at_ms, derivation_run_id, created_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(&draft.title)
        .bind(&draft.summary)
        .bind(draft.category.map(category_to_str))
        .bind(draft.started_at_ms)
        .bind(draft.ended_at_ms)
        .bind(draft.derivation_run_id)
        .bind(created_at_ms)
        .execute(&mut *transaction)
        .await?
        .last_insert_rowid();

        for evidence in &draft.evidence {
            sqlx::query(
                "INSERT OR IGNORE INTO user_context_activity_evidence \
                    (activity_id, subject_type, subject_id, captured_at_ms) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(activity_id)
            .bind(&evidence.subject_type)
            .bind(evidence.subject_id)
            .bind(evidence.captured_at_ms)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(activity_id)
    }

    /// Lists recent Activities newest-first (by `started_at_ms`), each hydrated
    /// with its evidence refs.
    pub async fn list_recent_activities(&self, limit: i64, offset: i64) -> Result<Vec<Activity>> {
        let rows = sqlx::query(
            "SELECT id, title, summary, category, started_at_ms, ended_at_ms, created_at_ms \
             FROM user_context_activities \
             ORDER BY started_at_ms DESC, id DESC \
             LIMIT ?1 OFFSET ?2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let mut activities = Vec::with_capacity(rows.len());
        for row in rows {
            let mut activity = map_activity(row);
            activity.evidence = self.list_activity_evidence(activity.id).await?;
            activities.push(activity);
        }
        Ok(activities)
    }

    async fn list_activity_evidence(&self, activity_id: i64) -> Result<Vec<ActivityEvidenceRef>> {
        let rows = sqlx::query(
            "SELECT subject_type, subject_id, captured_at_ms \
             FROM user_context_activity_evidence \
             WHERE activity_id = ?1 \
             ORDER BY captured_at_ms ASC, id ASC",
        )
        .bind(activity_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| ActivityEvidenceRef {
                subject_type: row.get("subject_type"),
                subject_id: row.get("subject_id"),
                captured_at_ms: row.get("captured_at_ms"),
            })
            .collect())
    }

    /// Total number of derived Activities.
    pub async fn count_activities(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS count FROM user_context_activities")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get("count"))
    }

    // --- #93: Derivation runs ---------------------------------------------

    /// Inserts a derivation-run ledger row, returning its id.
    pub async fn insert_derivation_run(&self, run: NewDerivationRun) -> Result<i64> {
        let created_at_ms = now_ms();
        let id = sqlx::query(
            "INSERT INTO user_context_derivation_runs \
                (kind, window_start_ms, window_end_ms, status, activities_derived, \
                 conclusions_derived, input_tokens, output_tokens, provider, model, error, created_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .bind(&run.kind)
        .bind(run.window_start_ms)
        .bind(run.window_end_ms)
        .bind(&run.status)
        .bind(run.activities_derived)
        .bind(run.conclusions_derived)
        .bind(run.input_tokens)
        .bind(run.output_tokens)
        .bind(run.provider.as_deref())
        .bind(run.model.as_deref())
        .bind(run.error.as_deref())
        .bind(created_at_ms)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    /// Records the (estimated) token usage on an existing derivation run, e.g.
    /// after the LLM round trip completes.
    pub async fn record_derivation_run_tokens(
        &self,
        run_id: i64,
        input_tokens: i64,
        output_tokens: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE user_context_derivation_runs \
             SET input_tokens = ?2, output_tokens = ?3 \
             WHERE id = ?1",
        )
        .bind(run_id)
        .bind(input_tokens)
        .bind(output_tokens)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The `(window_start_ms, window_end_ms)` of the most-recently-covered
    /// window (max `window_end_ms`), for newest-first / skip-already-derived
    /// scheduling. `None` if nothing has been derived (or no run carried a
    /// window).
    pub async fn latest_derivation_run_window(&self) -> Result<Option<(i64, i64)>> {
        let row = sqlx::query(
            "SELECT window_start_ms, window_end_ms \
             FROM user_context_derivation_runs \
             WHERE window_start_ms IS NOT NULL AND window_end_ms IS NOT NULL \
             ORDER BY window_end_ms DESC, id DESC \
             LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| {
            (
                row.get::<i64, _>("window_start_ms"),
                row.get::<i64, _>("window_end_ms"),
            )
        }))
    }

    /// Whether a derivation run already covers exactly `[start_ms, end_ms]`.
    pub async fn derived_window_exists(&self, start_ms: i64, end_ms: i64) -> Result<bool> {
        let row = sqlx::query(
            "SELECT EXISTS (\
                SELECT 1 FROM user_context_derivation_runs \
                WHERE window_start_ms = ?1 AND window_end_ms = ?2\
             ) AS found",
        )
        .bind(start_ms)
        .bind(end_ms)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("found") != 0)
    }

    // --- Token-usage / last-derived readouts ------------------------------

    /// Aggregated (estimated) token usage across every derivation run.
    pub async fn token_usage_totals(&self) -> Result<UserContextTokenUsage> {
        let row = sqlx::query(
            "SELECT \
                COALESCE(SUM(input_tokens), 0) AS input_tokens, \
                COALESCE(SUM(output_tokens), 0) AS output_tokens, \
                COUNT(*) AS run_count \
             FROM user_context_derivation_runs",
        )
        .fetch_one(&self.pool)
        .await?;

        let input_tokens: i64 = row.get("input_tokens");
        let output_tokens: i64 = row.get("output_tokens");
        Ok(UserContextTokenUsage {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens.saturating_add(output_tokens),
            run_count: row.get("run_count"),
        })
    }

    /// The most-recent successful derivation time (the `created_at_ms` of the
    /// newest `completed` run), for the status surface. `None` if nothing has
    /// completed yet.
    pub async fn last_derived_at_ms(&self) -> Result<Option<i64>> {
        let row = sqlx::query(
            "SELECT created_at_ms \
             FROM user_context_derivation_runs \
             WHERE status = 'completed' \
             ORDER BY created_at_ms DESC, id DESC \
             LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| row.get::<i64, _>("created_at_ms")))
    }

    /// The pool handle, for the capture-window reader (`capture_source.rs`).
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // later-slice: conclusion / confidence / dismissal / pin / wipe / cascade
    // methods land with migrations 0023–0026 in their own slices.
}

/// "Now" in unix milliseconds, derived from `time` (no `Date.now()`-style
/// nondeterminism).
pub(crate) fn now_ms() -> i64 {
    (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// The stored snake_case string for an [`ActivityCategory`] (matches the
/// capture-types serde rename).
fn category_to_str(category: ActivityCategory) -> &'static str {
    match category {
        ActivityCategory::Coding => "coding",
        ActivityCategory::Research => "research",
        ActivityCategory::Communication => "communication",
        ActivityCategory::Design => "design",
        ActivityCategory::Testing => "testing",
        ActivityCategory::Personal => "personal",
        ActivityCategory::Distractions => "distractions",
    }
}

/// Parses a stored category string back to an [`ActivityCategory`]; unknown /
/// NULL values map to `None`.
fn category_from_str(value: Option<&str>) -> Option<ActivityCategory> {
    match value {
        Some("coding") => Some(ActivityCategory::Coding),
        Some("research") => Some(ActivityCategory::Research),
        Some("communication") => Some(ActivityCategory::Communication),
        Some("design") => Some(ActivityCategory::Design),
        Some("testing") => Some(ActivityCategory::Testing),
        Some("personal") => Some(ActivityCategory::Personal),
        Some("distractions") => Some(ActivityCategory::Distractions),
        _ => None,
    }
}

fn map_activity(row: SqliteRow) -> Activity {
    let category: Option<String> = row.get("category");
    Activity {
        id: row.get("id"),
        title: row.get("title"),
        summary: row.get("summary"),
        category: category_from_str(category.as_deref()),
        started_at_ms: row.get("started_at_ms"),
        ended_at_ms: row.get("ended_at_ms"),
        created_at_ms: row.get("created_at_ms"),
        evidence: Vec::new(),
    }
}
