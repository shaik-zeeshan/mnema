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

use capture_types::{
    Activity, ActivityCategory, ActivityEvidenceRef, Conclusion, ConclusionEvidenceRef,
    ConclusionStatus, EvidenceStance, UserContextTokenUsage,
};

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

/// A new (or to-be-updated) **Conclusion** ready to persist via
/// [`UserContextStore::upsert_conclusion`]. A Conclusion is open-ended
/// natural language (a `subject` it is about + a plain-language `statement`),
/// not a fixed subject+attribute+value row.
#[derive(Debug, Clone, PartialEq)]
pub struct NewConclusion {
    pub subject: String,
    pub statement: String,
    pub confidence: f64,
    pub formed_at_ms: i64,
    pub last_supported_at_ms: i64,
}

/// One evidence link for a [`NewConclusion`]: an [`Activity`] id plus the
/// stance (support / contradict) it lends the Conclusion. Persisted via
/// [`UserContextStore::replace_conclusion_evidence`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewConclusionEvidence {
    pub activity_id: i64,
    pub stance: EvidenceStance,
}

/// SQLite-backed storage for the User Context dossier (Activities + evidence +
/// derivation runs + Conclusions in this slice).
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

    // --- #94: Conclusions + evidence --------------------------------------

    /// Insert or update a **Conclusion**. The dedup key is the case-insensitive
    /// `(subject, statement)` pair: if a matching row exists, its confidence,
    /// `last_supported_at_ms`, and `updated_at_ms` are refreshed and its id
    /// returned; otherwise a new `visible` row is inserted (with
    /// `created_at_ms`/`updated_at_ms` = now) and the new id returned.
    pub async fn upsert_conclusion(&self, draft: NewConclusion) -> Result<i64> {
        let now = now_ms();

        // Case-insensitive dedup on (subject, statement). NOCASE collation is
        // ASCII-only, which matches the rest of the store's matching.
        let existing = sqlx::query(
            "SELECT id FROM user_context_conclusions \
             WHERE subject = ?1 COLLATE NOCASE AND statement = ?2 COLLATE NOCASE \
             ORDER BY id ASC LIMIT 1",
        )
        .bind(&draft.subject)
        .bind(&draft.statement)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = existing {
            let id: i64 = row.get("id");
            sqlx::query(
                "UPDATE user_context_conclusions \
                 SET confidence = ?2, last_supported_at_ms = ?3, updated_at_ms = ?4 \
                 WHERE id = ?1",
            )
            .bind(id)
            .bind(draft.confidence)
            .bind(draft.last_supported_at_ms)
            .bind(now)
            .execute(&self.pool)
            .await?;
            return Ok(id);
        }

        let id = sqlx::query(
            "INSERT INTO user_context_conclusions \
                (subject, statement, confidence, status, formed_at_ms, \
                 last_supported_at_ms, updated_at_ms, created_at_ms) \
             VALUES (?1, ?2, ?3, 'visible', ?4, ?5, ?6, ?6)",
        )
        .bind(&draft.subject)
        .bind(&draft.statement)
        .bind(draft.confidence)
        .bind(draft.formed_at_ms)
        .bind(draft.last_supported_at_ms)
        .bind(now)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    /// Replace the full evidence set for a Conclusion: delete its existing
    /// evidence rows, then insert the new set (`INSERT OR IGNORE` on the
    /// `UNIQUE(conclusion_id, activity_id)`), in one transaction.
    pub async fn replace_conclusion_evidence(
        &self,
        conclusion_id: i64,
        evidence: Vec<NewConclusionEvidence>,
    ) -> Result<()> {
        let created_at_ms = now_ms();
        let mut transaction = self.pool.begin().await?;

        sqlx::query("DELETE FROM user_context_conclusion_evidence WHERE conclusion_id = ?1")
            .bind(conclusion_id)
            .execute(&mut *transaction)
            .await?;

        for link in &evidence {
            sqlx::query(
                "INSERT OR IGNORE INTO user_context_conclusion_evidence \
                    (conclusion_id, activity_id, stance, created_at_ms) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(conclusion_id)
            .bind(link.activity_id)
            .bind(stance_to_str(link.stance))
            .bind(created_at_ms)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(())
    }

    /// The most-recent Activities (by `started_at_ms`) for a Conclusion
    /// distillation pass, each hydrated with its evidence, capped at `limit`.
    pub async fn activities_for_distillation(&self, limit: i64) -> Result<Vec<Activity>> {
        let rows = sqlx::query(
            "SELECT id, title, summary, category, started_at_ms, ended_at_ms, created_at_ms \
             FROM user_context_activities \
             ORDER BY started_at_ms DESC, id DESC \
             LIMIT ?1",
        )
        .bind(limit)
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

    /// List Conclusions for the dossier preview. `visible` rows are always
    /// included; `faded` rows are included only when `include_faded`; `dismissed`
    /// rows are NEVER returned. Ordered by confidence DESC, then recency.
    pub async fn list_conclusions(&self, include_faded: bool) -> Result<Vec<Conclusion>> {
        let sql = if include_faded {
            "SELECT id, subject, statement, confidence, status, formed_at_ms, \
                    last_supported_at_ms, updated_at_ms \
             FROM user_context_conclusions \
             WHERE status IN ('visible', 'faded') \
             ORDER BY confidence DESC, updated_at_ms DESC, id DESC"
        } else {
            "SELECT id, subject, statement, confidence, status, formed_at_ms, \
                    last_supported_at_ms, updated_at_ms \
             FROM user_context_conclusions \
             WHERE status = 'visible' \
             ORDER BY confidence DESC, updated_at_ms DESC, id DESC"
        };

        let rows = sqlx::query(sql).fetch_all(&self.pool).await?;
        self.hydrate_conclusions(rows).await
    }

    /// All non-dismissed Conclusions about a Subject (case-insensitive match),
    /// faded included, hydrated with their evidence. Powers the Subject page.
    pub async fn list_conclusions_for_subject(&self, subject: &str) -> Result<Vec<Conclusion>> {
        let rows = sqlx::query(
            "SELECT id, subject, statement, confidence, status, formed_at_ms, \
                    last_supported_at_ms, updated_at_ms \
             FROM user_context_conclusions \
             WHERE subject = ?1 COLLATE NOCASE AND status != 'dismissed' \
             ORDER BY confidence DESC, updated_at_ms DESC, id DESC",
        )
        .bind(subject)
        .fetch_all(&self.pool)
        .await?;
        self.hydrate_conclusions(rows).await
    }

    /// Number of non-dismissed Conclusions (the status-surface count).
    pub async fn count_conclusions(&self) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS count FROM user_context_conclusions WHERE status != 'dismissed'",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("count"))
    }

    /// Fetch one Conclusion by id, hydrated with its evidence. `None` if absent.
    pub async fn get_conclusion(&self, id: i64) -> Result<Option<Conclusion>> {
        let row = sqlx::query(
            "SELECT id, subject, statement, confidence, status, formed_at_ms, \
                    last_supported_at_ms, updated_at_ms \
             FROM user_context_conclusions \
             WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let mut conclusion = map_conclusion(row);
                conclusion.evidence = self.list_conclusion_evidence(conclusion.id).await?;
                Ok(Some(conclusion))
            }
            None => Ok(None),
        }
    }

    /// Hydrate a batch of conclusion rows with their evidence refs.
    async fn hydrate_conclusions(&self, rows: Vec<SqliteRow>) -> Result<Vec<Conclusion>> {
        let mut conclusions = Vec::with_capacity(rows.len());
        for row in rows {
            let mut conclusion = map_conclusion(row);
            conclusion.evidence = self.list_conclusion_evidence(conclusion.id).await?;
            conclusions.push(conclusion);
        }
        Ok(conclusions)
    }

    /// Evidence refs for one Conclusion, joined to their Activity for the title +
    /// start time the dossier surface shows alongside each link.
    async fn list_conclusion_evidence(
        &self,
        conclusion_id: i64,
    ) -> Result<Vec<ConclusionEvidenceRef>> {
        let rows = sqlx::query(
            "SELECT ce.activity_id AS activity_id, ce.stance AS stance, \
                    a.title AS activity_title, a.started_at_ms AS activity_started_at_ms \
             FROM user_context_conclusion_evidence ce \
             LEFT JOIN user_context_activities a ON a.id = ce.activity_id \
             WHERE ce.conclusion_id = ?1 \
             ORDER BY a.started_at_ms ASC, ce.id ASC",
        )
        .bind(conclusion_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let stance: String = row.get("stance");
                ConclusionEvidenceRef {
                    activity_id: row.get("activity_id"),
                    stance: stance_from_str(&stance),
                    activity_title: row.get("activity_title"),
                    activity_started_at_ms: row.get("activity_started_at_ms"),
                }
            })
            .collect())
    }

    /// The pool handle, for the capture-window reader (`capture_source.rs`).
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // later-slice: confidence / dismissal / pin / wipe / cascade methods land
    // with migrations 0024–0026 in their own slices.
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

/// The stored snake_case string for an [`EvidenceStance`] (matches the
/// capture-types serde rename and the SQL `stance` column).
fn stance_to_str(stance: EvidenceStance) -> &'static str {
    match stance {
        EvidenceStance::Support => "support",
        EvidenceStance::Contradict => "contradict",
    }
}

/// Parse a stored `stance` column value; unknown values fall back to `Support`
/// (the column default), matching the over-conservative store posture.
fn stance_from_str(value: &str) -> EvidenceStance {
    match value {
        "contradict" => EvidenceStance::Contradict,
        _ => EvidenceStance::Support,
    }
}

/// Parse a stored `status` column value back to a [`ConclusionStatus`]. Unknown
/// values fall back to `Visible`. (Note: `list_conclusions` already filters out
/// `dismissed` rows, so a mapped value is normally `Visible`/`Faded`.)
fn status_from_str(value: &str) -> ConclusionStatus {
    match value {
        "faded" => ConclusionStatus::Faded,
        "dismissed" => ConclusionStatus::Dismissed,
        _ => ConclusionStatus::Visible,
    }
}

fn map_conclusion(row: SqliteRow) -> Conclusion {
    let status: String = row.get("status");
    Conclusion {
        id: row.get("id"),
        subject: row.get("subject"),
        statement: row.get("statement"),
        confidence: row.get("confidence"),
        status: status_from_str(&status),
        // TODO(#99): read pinned column. The `pinned` column lands in migration
        // 0026 (#99); until then every Conclusion maps as unpinned.
        pinned: false,
        formed_at_ms: row.get("formed_at_ms"),
        last_supported_at_ms: row.get("last_supported_at_ms"),
        updated_at_ms: row.get("updated_at_ms"),
        evidence: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Run an async test body on a current-thread runtime (the crate's
    /// `tokio` dep does not enable the `macros` feature, so there is no
    /// `#[tokio::test]`; this mirrors `capture_retention.rs`'s test pattern).
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    /// An in-memory store with just the user_context tables this slice needs.
    async fn test_store() -> UserContextStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory db should open");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("enable foreign keys");
        for statement in [
            "CREATE TABLE user_context_activities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                category TEXT,
                started_at_ms INTEGER NOT NULL,
                ended_at_ms INTEGER NOT NULL,
                derivation_run_id INTEGER,
                created_at_ms INTEGER NOT NULL
            )",
            "CREATE TABLE user_context_activity_evidence (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                activity_id INTEGER NOT NULL REFERENCES user_context_activities(id) ON DELETE CASCADE,
                subject_type TEXT NOT NULL,
                subject_id INTEGER NOT NULL,
                captured_at_ms INTEGER,
                UNIQUE (activity_id, subject_type, subject_id)
            )",
            "CREATE TABLE user_context_conclusions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subject TEXT NOT NULL,
                statement TEXT NOT NULL,
                confidence REAL NOT NULL,
                status TEXT NOT NULL DEFAULT 'visible',
                formed_at_ms INTEGER NOT NULL,
                last_supported_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                created_at_ms INTEGER NOT NULL
            )",
            "CREATE TABLE user_context_conclusion_evidence (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conclusion_id INTEGER NOT NULL REFERENCES user_context_conclusions(id) ON DELETE CASCADE,
                activity_id INTEGER NOT NULL REFERENCES user_context_activities(id) ON DELETE CASCADE,
                stance TEXT NOT NULL DEFAULT 'support',
                created_at_ms INTEGER NOT NULL,
                UNIQUE (conclusion_id, activity_id)
            )",
        ] {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("create user_context table");
        }
        UserContextStore::new(pool)
    }

    async fn seed_activity(store: &UserContextStore, title: &str, started_at_ms: i64) -> i64 {
        store
            .insert_activity_with_evidence(NewActivity {
                title: title.to_string(),
                summary: format!("{title} summary"),
                category: None,
                started_at_ms,
                ended_at_ms: started_at_ms + 1,
                derivation_run_id: None,
                evidence: vec![NewActivityEvidence {
                    subject_type: "frame".to_string(),
                    subject_id: started_at_ms,
                    captured_at_ms: Some(started_at_ms),
                }],
            })
            .await
            .expect("insert activity")
    }

    fn draft(subject: &str, statement: &str, confidence: f64) -> NewConclusion {
        NewConclusion {
            subject: subject.to_string(),
            statement: statement.to_string(),
            confidence,
            formed_at_ms: 1_000,
            last_supported_at_ms: 1_000,
        }
    }

    #[test]
    fn upsert_conclusion_dedups_case_insensitively() {
        block_on(async {
        let store = test_store().await;
        let first = store
            .upsert_conclusion(draft("Apple", "Interested in Apple", 0.4))
            .await
            .expect("first upsert");
        // Same subject+statement with different casing => UPDATE, same id.
        let second = store
            .upsert_conclusion(draft("apple", "interested in apple", 0.7))
            .await
            .expect("second upsert");
        assert_eq!(first, second, "case-insensitive dedup should reuse the id");

        let conclusion = store
            .get_conclusion(first)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(conclusion.confidence, 0.7, "confidence should be refreshed");
        assert_eq!(store.count_conclusions().await.expect("count"), 1);
        });
    }

    #[test]
    fn replace_conclusion_evidence_hydrates_with_stance_and_title() {
        block_on(async {
        let store = test_store().await;
        let support = seed_activity(&store, "Read Apple news", 100).await;
        let contradict = seed_activity(&store, "Bought a Pixel", 200).await;
        let id = store
            .upsert_conclusion(draft("Apple", "Warming up to Apple", 0.5))
            .await
            .expect("upsert");

        store
            .replace_conclusion_evidence(
                id,
                vec![
                    NewConclusionEvidence {
                        activity_id: support,
                        stance: EvidenceStance::Support,
                    },
                    NewConclusionEvidence {
                        activity_id: contradict,
                        stance: EvidenceStance::Contradict,
                    },
                ],
            )
            .await
            .expect("replace evidence");

        let conclusion = store
            .get_conclusion(id)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(conclusion.evidence.len(), 2);
        // Ordered by activity started_at_ms ASC.
        assert_eq!(conclusion.evidence[0].activity_id, support);
        assert_eq!(conclusion.evidence[0].stance, EvidenceStance::Support);
        assert_eq!(
            conclusion.evidence[0].activity_title.as_deref(),
            Some("Read Apple news")
        );
        assert_eq!(conclusion.evidence[1].stance, EvidenceStance::Contradict);

        // Replacing with a single ref drops the others.
        store
            .replace_conclusion_evidence(
                id,
                vec![NewConclusionEvidence {
                    activity_id: support,
                    stance: EvidenceStance::Support,
                }],
            )
            .await
            .expect("replace evidence again");
        let conclusion = store
            .get_conclusion(id)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(conclusion.evidence.len(), 1);
        });
    }

    #[test]
    fn list_conclusions_respects_status_filter() {
        block_on(async {
        let store = test_store().await;
        let visible = store
            .upsert_conclusion(draft("Rust", "Learning Rust", 0.8))
            .await
            .expect("visible");
        let faded = store
            .upsert_conclusion(draft("Vim", "Used Vim once", 0.2))
            .await
            .expect("faded");
        let dismissed = store
            .upsert_conclusion(draft("Coffee", "Hates coffee", 0.6))
            .await
            .expect("dismissed");
        // Hand-set statuses (the dedicated setters land in #95/#99).
        sqlx::query("UPDATE user_context_conclusions SET status = 'faded' WHERE id = ?1")
            .bind(faded)
            .execute(store.pool())
            .await
            .expect("set faded");
        sqlx::query("UPDATE user_context_conclusions SET status = 'dismissed' WHERE id = ?1")
            .bind(dismissed)
            .execute(store.pool())
            .await
            .expect("set dismissed");

        let visible_only = store.list_conclusions(false).await.expect("visible only");
        assert_eq!(visible_only.len(), 1);
        assert_eq!(visible_only[0].id, visible);

        let with_faded = store.list_conclusions(true).await.expect("with faded");
        assert_eq!(with_faded.len(), 2, "faded included, dismissed never");
        // Ordered by confidence DESC.
        assert_eq!(with_faded[0].id, visible);
        assert_eq!(with_faded[1].id, faded);

        // count_conclusions excludes dismissed only.
        assert_eq!(store.count_conclusions().await.expect("count"), 2);
        });
    }
}
