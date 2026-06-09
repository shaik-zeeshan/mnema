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

use sqlx::{sqlite::SqliteRow, QueryBuilder, Row, Sqlite, SqlitePool};
use time::OffsetDateTime;

use capture_types::{
    Activity, ActivityCategory, ActivityEvidenceRef, Conclusion, ConclusionEvidenceRef,
    ConclusionStatus, ConfidenceSnapshot, DismissalState, EvidenceStance, UserContextTokenUsage,
};

use crate::Result;

/// Max bound parameters per `IN (...)` chunk, mirroring
/// `capture_retention.rs`'s `SQLITE_BIND_CHUNK_SIZE` so large delete-subject
/// id lists stay well under SQLite's bind limit.
const SQLITE_BIND_CHUNK_SIZE: usize = 500;

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

/// Counts from a **Delete Recent Capture** derived-data cascade
/// ([`UserContextStore::delete_derived_for_capture_subjects`]): how many
/// **Activity** rows and how many now-ungrounded **Conclusion** rows were
/// dropped. Used for the warning log + UI refresh; not persisted.
#[derive(Debug, Clone, Default)]
pub struct UserContextCascadeSummary {
    pub deleted_activities: i64,
    pub deleted_conclusions: i64,
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

    /// The OLDEST `window_start_ms` covered by a windowed (`'activity'` /
    /// `'backfill'`) derivation run — the trailing edge of coverage that the
    /// **History Backfill** (#98) extends backward, newest-first. `None` when
    /// nothing windowed has run yet (the forward pass seeds coverage first).
    ///
    /// Only `activity`/`backfill` runs carry window bounds; `conclusion` /
    /// `confidence` runs have NULL bounds and are excluded by the IS NOT NULL
    /// filter anyway.
    pub async fn oldest_derivation_run_window_start(&self) -> Result<Option<i64>> {
        let row = sqlx::query(
            "SELECT MIN(window_start_ms) AS oldest \
             FROM user_context_derivation_runs \
             WHERE window_start_ms IS NOT NULL \
               AND kind IN ('activity', 'backfill')",
        )
        .fetch_one(&self.pool)
        .await?;
        // MIN over an empty/all-NULL set is SQL NULL → read as an Option column.
        Ok(row.get::<Option<i64>, _>("oldest"))
    }

    /// The earliest captured-at across all raw captures, in unix millis — the
    /// true history floor that **go-deeper** backfill walks toward. Takes the
    /// MIN of `frames.captured_at` and `audio_segments.started_at` (both legacy
    /// RFC3339 TEXT), converting at the boundary exactly as `read_capture_window`
    /// does. `None` when there are no captures at all.
    pub async fn earliest_capture_at_ms(&self) -> Result<Option<i64>> {
        // RFC3339 TEXT sorts lexicographically in captured order for a fixed
        // zone/format, but we never rely on that across the two tables: read the
        // MIN TEXT from each table independently, parse each to millis, and take
        // the smaller. Parse failures fall through to the other source.
        let frame_min: Option<String> = sqlx::query("SELECT MIN(captured_at) AS m FROM frames")
            .fetch_one(&self.pool)
            .await?
            .get::<Option<String>, _>("m");
        let audio_min: Option<String> =
            sqlx::query("SELECT MIN(started_at) AS m FROM audio_segments")
                .fetch_one(&self.pool)
                .await?
                .get::<Option<String>, _>("m");

        let frame_ms = frame_min.as_deref().and_then(rfc3339_text_to_ms);
        let audio_ms = audio_min.as_deref().and_then(rfc3339_text_to_ms);

        Ok(match (frame_ms, audio_ms) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        })
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
            "SELECT id, subject, statement, confidence, status, pinned, formed_at_ms, \
                    last_supported_at_ms, updated_at_ms \
             FROM user_context_conclusions \
             WHERE status IN ('visible', 'faded') \
             ORDER BY confidence DESC, updated_at_ms DESC, id DESC"
        } else {
            "SELECT id, subject, statement, confidence, status, pinned, formed_at_ms, \
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
            "SELECT id, subject, statement, confidence, status, pinned, formed_at_ms, \
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
            "SELECT id, subject, statement, confidence, status, pinned, formed_at_ms, \
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

    // --- #95: Confidence History + decay bookkeeping ----------------------

    /// Append one **Confidence History** snapshot for a Conclusion. Snapshots are
    /// the time-series that powers the Subject trajectory line; they are tiny and
    /// aggressively prunable (see [`prune_confidence_history`]).
    pub async fn insert_confidence_snapshot(
        &self,
        conclusion_id: i64,
        confidence: f64,
        at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO user_context_confidence_history \
                (conclusion_id, confidence, snapshot_at_ms) \
             VALUES (?1, ?2, ?3)",
        )
        .bind(conclusion_id)
        .bind(confidence)
        .bind(at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The full **Confidence History** for one Conclusion, oldest snapshot first
    /// (ascending `snapshot_at_ms`) so the Subject page can plot the trajectory.
    pub async fn list_confidence_history(
        &self,
        conclusion_id: i64,
    ) -> Result<Vec<ConfidenceSnapshot>> {
        let rows = sqlx::query(
            "SELECT confidence, snapshot_at_ms \
             FROM user_context_confidence_history \
             WHERE conclusion_id = ?1 \
             ORDER BY snapshot_at_ms ASC, id ASC",
        )
        .bind(conclusion_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| ConfidenceSnapshot {
                confidence: row.get("confidence"),
                snapshot_at_ms: row.get("snapshot_at_ms"),
            })
            .collect())
    }

    /// Prune **Confidence History** to the newest `max_per_conclusion` snapshots
    /// per Conclusion, deleting older ones. Confidence History is aggressively
    /// prunable: recency-weighting means old snapshots stop mattering, so the
    /// trajectory keeps only its recent tail. Returns the number of rows deleted.
    ///
    /// `max_per_conclusion <= 0` is treated as "keep nothing" and deletes all
    /// history (an explicit caller intent, not the worker's path — the worker
    /// passes a positive cap).
    pub async fn prune_confidence_history(&self, max_per_conclusion: i64) -> Result<u64> {
        // Delete every snapshot that is NOT among the newest `max_per_conclusion`
        // for its Conclusion. The subquery ranks each Conclusion's snapshots
        // newest-first; rows ranked beyond the cap are removed.
        let result = sqlx::query(
            "DELETE FROM user_context_confidence_history \
             WHERE id IN (\
                 SELECT id FROM (\
                     SELECT id, \
                            ROW_NUMBER() OVER (\
                                PARTITION BY conclusion_id \
                                ORDER BY snapshot_at_ms DESC, id DESC\
                            ) AS rn \
                     FROM user_context_confidence_history\
                 ) ranked \
                 WHERE ranked.rn > ?1\
             )",
        )
        .bind(max_per_conclusion.max(0))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Persist a decayed confidence + recomputed visibility status for a
    /// Conclusion and stamp `last_decayed_at_ms` (the decay-beat bookkeeping
    /// column). Also bumps `updated_at_ms` so the surface re-sorts. Used by the
    /// confidence-decay beat (#95).
    pub async fn update_conclusion_confidence(
        &self,
        id: i64,
        confidence: f64,
        status: ConclusionStatus,
        last_decayed_at_ms: i64,
    ) -> Result<()> {
        let now = now_ms();
        sqlx::query(
            "UPDATE user_context_conclusions \
             SET confidence = ?2, status = ?3, last_decayed_at_ms = ?4, updated_at_ms = ?5 \
             WHERE id = ?1",
        )
        .bind(id)
        .bind(confidence)
        .bind(status_to_str(status))
        .bind(last_decayed_at_ms)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Set a Conclusion's visibility status directly (bumping `updated_at_ms`).
    /// A thin setter used where only the status changes (the confidence-decay
    /// beat uses [`update_conclusion_confidence`]).
    pub async fn set_conclusion_status(
        &self,
        id: i64,
        status: ConclusionStatus,
    ) -> Result<()> {
        let now = now_ms();
        sqlx::query(
            "UPDATE user_context_conclusions \
             SET status = ?2, updated_at_ms = ?3 \
             WHERE id = ?1",
        )
        .bind(id)
        .bind(status_to_str(status))
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Conclusions eligible for the confidence-decay beat: `visible` or `faded`
    /// (dismissed Conclusions are out of the dossier) and **not pinned** — a Pin
    /// exempts a Conclusion from confidence decay, so a pinned row is dropped from
    /// the decayable set entirely. Hydrated with their evidence. Ordered
    /// oldest-supported-first so the loop touches the stalest rows first.
    pub async fn list_decayable_conclusions(&self) -> Result<Vec<Conclusion>> {
        let rows = sqlx::query(
            "SELECT id, subject, statement, confidence, status, pinned, formed_at_ms, \
                    last_supported_at_ms, updated_at_ms \
             FROM user_context_conclusions \
             WHERE status IN ('visible', 'faded') AND COALESCE(pinned, 0) = 0 \
             ORDER BY last_supported_at_ms ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        self.hydrate_conclusions(rows).await
    }

    // --- #99: Pin + Dismiss + Dismissal State -----------------------------

    /// Set (or clear) a Conclusion's **Pin** flag. A pinned Conclusion is exempt
    /// from confidence decay (it is dropped from `list_decayable_conclusions` and
    /// `confidence::decay`/`status_for` already honor `pinned`). Bumps
    /// `updated_at_ms` so the dossier re-sorts.
    pub async fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        let now = now_ms();
        sqlx::query(
            "UPDATE user_context_conclusions \
             SET pinned = ?2, updated_at_ms = ?3 \
             WHERE id = ?1",
        )
        .bind(id)
        .bind(if pinned { 1_i64 } else { 0_i64 })
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// **Dismiss** a Conclusion: in ONE transaction, record its **Dismissal
    /// State** (which evidence, when) and then remove the Conclusion. The
    /// dismissal row OUTLIVES the deleted Conclusion (no FK to it) and is fed to
    /// every future derivation pass so the engine can tell fresh evidence from the
    /// evidence already vetoed and honor the high resurface bar. A no-op (and no
    /// dismissal row) when `id` names no Conclusion.
    ///
    /// The recorded `evidence_fingerprint` is the deterministic
    /// [`evidence_fingerprint`] of the Conclusion's distinct evidence Activity ids
    /// (all stances), and `evidence_activity_count` is the count of its
    /// support-stance evidence — the baseline the resurface bar is measured
    /// against. Deleting the Conclusion cascades its evidence + confidence-history
    /// rows via FK.
    pub async fn dismiss_conclusion(&self, id: i64) -> Result<()> {
        let now = now_ms();
        let mut transaction = self.pool.begin().await?;

        // Load the Conclusion's subject/statement; bail (no dismissal) if absent.
        let conclusion = sqlx::query(
            "SELECT subject, statement FROM user_context_conclusions WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(conclusion) = conclusion else {
            transaction.commit().await?;
            return Ok(());
        };
        let subject: String = conclusion.get("subject");
        let statement: String = conclusion.get("statement");

        // The full distinct evidence activity-id set (any stance) → fingerprint, so
        // the same evidence just rejected can never resurface the Conclusion.
        let evidence_rows = sqlx::query(
            "SELECT activity_id FROM user_context_conclusion_evidence WHERE conclusion_id = ?1",
        )
        .bind(id)
        .fetch_all(&mut *transaction)
        .await?;
        let evidence_ids: Vec<i64> =
            evidence_rows.iter().map(|row| row.get("activity_id")).collect();
        let fingerprint = evidence_fingerprint(&evidence_ids);

        // The support-stance count is the baseline the high resurface bar measures
        // fresh support against (a Dismiss needs substantially MORE to overturn).
        let support_count: i64 = sqlx::query(
            "SELECT COUNT(*) AS count FROM user_context_conclusion_evidence \
             WHERE conclusion_id = ?1 AND stance = 'support'",
        )
        .bind(id)
        .fetch_one(&mut *transaction)
        .await?
        .get("count");

        sqlx::query(
            "INSERT INTO user_context_dismissals \
                (subject, statement, evidence_fingerprint, evidence_activity_count, dismissed_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&subject)
        .bind(&statement)
        .bind(&fingerprint)
        .bind(support_count)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        // Remove the Conclusion; its evidence + confidence-history cascade via FK.
        sqlx::query("DELETE FROM user_context_conclusions WHERE id = ?1")
            .bind(id)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(())
    }

    /// Every recorded **Dismissal State**, newest first. Fed to the derivation
    /// pass so it can avoid reconstituting dismissed Conclusions (and so the
    /// resurface gate can compare fresh evidence to what was vetoed).
    pub async fn list_dismissals(&self) -> Result<Vec<DismissalState>> {
        let rows = sqlx::query(
            "SELECT subject, statement, evidence_fingerprint, evidence_activity_count, dismissed_at_ms \
             FROM user_context_dismissals \
             ORDER BY dismissed_at_ms DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(map_dismissal).collect())
    }

    /// Recorded **Dismissal State** for one Subject (case-insensitive), newest
    /// first.
    pub async fn list_dismissals_for_subject(
        &self,
        subject: &str,
    ) -> Result<Vec<DismissalState>> {
        let rows = sqlx::query(
            "SELECT subject, statement, evidence_fingerprint, evidence_activity_count, dismissed_at_ms \
             FROM user_context_dismissals \
             WHERE subject = ?1 COLLATE NOCASE \
             ORDER BY dismissed_at_ms DESC, id DESC",
        )
        .bind(subject)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(map_dismissal).collect())
    }

    /// The pool handle, for the capture-window reader (`capture_source.rs`).
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // --- #97: Delete Recent Capture cascade + Wipe User Context ------------

    /// **Delete Recent Capture** derived-data cascade (ADR 0029). The privacy
    /// panic button has just deleted the raw frames / audio segments named by
    /// `frame_ids` / `audio_ids`; this purges the **Activity** rows derived from
    /// them and drops any **Conclusion** left with no surviving evidence.
    ///
    /// In ONE transaction:
    /// 1. Find every Activity with ANY evidence row pointing at a deleted subject
    ///    (`subject_type='frame' AND subject_id IN frame_ids` OR
    ///    `subject_type='audio_segment' AND subject_id IN audio_ids`), chunked by
    ///    [`SQLITE_BIND_CHUNK_SIZE`].
    /// 2. DELETE those Activities — their `*_activity_evidence` and
    ///    `*_conclusion_evidence` link rows cascade via FK.
    /// 3. DROP every Conclusion that now has ZERO remaining
    ///    `*_conclusion_evidence` rows (no ungrounded Conclusions). A Conclusion
    ///    still grounded by ≥1 surviving evidence Activity STAYS — the minimal
    ///    "re-judge or drop" rule: drop ungrounded, keep grounded.
    ///
    /// **Dismissal State is left untouched**: a dismissal is keyed by
    /// subject/statement (no FK to any capture subject) and must survive capture
    /// deletion. Returns the dropped Activity / Conclusion counts. A no-op (and an
    /// empty summary) when both id lists are empty.
    pub async fn delete_derived_for_capture_subjects(
        &self,
        frame_ids: &[i64],
        audio_ids: &[i64],
    ) -> Result<UserContextCascadeSummary> {
        if frame_ids.is_empty() && audio_ids.is_empty() {
            return Ok(UserContextCascadeSummary::default());
        }

        let mut tx = self.pool.begin().await?;

        // 1. Activities with any evidence row in the deleted subjects.
        let mut activity_ids: Vec<i64> = Vec::new();
        for (subject_type, subject_ids) in
            [("frame", frame_ids), ("audio_segment", audio_ids)]
        {
            for chunk in subject_ids.chunks(SQLITE_BIND_CHUNK_SIZE) {
                if chunk.is_empty() {
                    continue;
                }
                let mut query = QueryBuilder::<Sqlite>::new(
                    "SELECT DISTINCT activity_id FROM user_context_activity_evidence \
                     WHERE subject_type = ",
                );
                query.push_bind(subject_type);
                query.push(" AND subject_id IN (");
                let mut separated = query.separated(", ");
                for id in chunk {
                    separated.push_bind(id);
                }
                separated.push_unseparated(")");
                activity_ids.extend(
                    query
                        .build()
                        .fetch_all(&mut *tx)
                        .await?
                        .into_iter()
                        .map(|row| row.get::<i64, _>("activity_id")),
                );
            }
        }
        activity_ids.sort_unstable();
        activity_ids.dedup();

        // 2. DELETE those Activities; activity_evidence + conclusion_evidence rows
        //    cascade via FK.
        let mut deleted_activities = 0_i64;
        for chunk in activity_ids.chunks(SQLITE_BIND_CHUNK_SIZE) {
            let mut query = QueryBuilder::<Sqlite>::new(
                "DELETE FROM user_context_activities WHERE id IN (",
            );
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            deleted_activities += query.build().execute(&mut *tx).await?.rows_affected() as i64;
        }

        // 3. Drop every Conclusion now grounded by ZERO evidence Activities (no
        //    ungrounded Conclusions). A Conclusion with ≥1 surviving evidence row
        //    stays.
        let deleted_conclusions = sqlx::query(
            "DELETE FROM user_context_conclusions \
             WHERE NOT EXISTS (\
                 SELECT 1 FROM user_context_conclusion_evidence ce \
                 WHERE ce.conclusion_id = user_context_conclusions.id\
             )",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;

        tx.commit().await?;
        Ok(UserContextCascadeSummary {
            deleted_activities,
            deleted_conclusions,
        })
    }

    /// **Wipe User Context** storage half (ADR 0029): in ONE transaction, clear
    /// every `user_context_*` table — all derived **Activity** / **Conclusion**
    /// data AND **Dismissal State** and the derivation-run ledger. Raw captures
    /// and settings are untouched (this only owns the dossier tables); the engine
    /// is turned off by the Tauri command, not here. Deletes children before
    /// parents to stay correct regardless of FK enforcement.
    pub async fn wipe_all(&self) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for table in [
            // Children first (leaf evidence / history), then parents, then the
            // FK-free dismissal + derivation-run ledgers.
            "user_context_activity_evidence",
            "user_context_conclusion_evidence",
            "user_context_confidence_history",
            "user_context_activities",
            "user_context_conclusions",
            "user_context_dismissals",
            "user_context_derivation_runs",
        ] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

/// Deterministic fingerprint of an evidence **Activity**-id set: the sorted,
/// distinct ids joined by `','` (an empty set → `""`). Used both when recording a
/// **Dismissal State** (the evidence a Conclusion was built on) and by the
/// derivation layer when re-deriving a Conclusion, so an identical evidence set
/// produces an identical fingerprint and the resurface gate can recognize "the
/// same evidence just rejected" exactly.
pub fn evidence_fingerprint(activity_ids: &[i64]) -> String {
    let mut ids: Vec<i64> = activity_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Map a `user_context_dismissals` row onto a [`DismissalState`].
fn map_dismissal(row: SqliteRow) -> DismissalState {
    DismissalState {
        subject: row.get("subject"),
        statement: row.get("statement"),
        evidence_fingerprint: row.get("evidence_fingerprint"),
        evidence_activity_count: row.get("evidence_activity_count"),
        dismissed_at_ms: row.get("dismissed_at_ms"),
    }
}

/// "Now" in unix milliseconds, derived from `time` (no `Date.now()`-style
/// nondeterminism).
pub(crate) fn now_ms() -> i64 {
    (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// Converts a legacy RFC3339 TEXT timestamp (`frames.captured_at` /
/// `audio_segments.started_at`) to unix milliseconds; `None` on a parse
/// failure. Mirrors `capture_source.rs`'s boundary conversion so
/// `earliest_capture_at_ms` and `read_capture_window` agree on the floor.
fn rfc3339_text_to_ms(value: &str) -> Option<i64> {
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .map(|dt| (dt.unix_timestamp_nanos() / 1_000_000) as i64)
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

/// The stored snake_case string for a [`ConclusionStatus`] (matches the
/// capture-types serde rename and the SQL `status` column).
fn status_to_str(status: ConclusionStatus) -> &'static str {
    match status {
        ConclusionStatus::Visible => "visible",
        ConclusionStatus::Faded => "faded",
        ConclusionStatus::Dismissed => "dismissed",
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
        // The `pinned` column (migration 0025, #99) is stored as INTEGER 0/1; map
        // any non-zero value to a pinned Conclusion (exempt from confidence decay).
        pinned: row.get::<i64, _>("pinned") != 0,
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
            "CREATE TABLE user_context_derivation_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                window_start_ms INTEGER,
                window_end_ms INTEGER,
                status TEXT NOT NULL DEFAULT 'completed',
                activities_derived INTEGER NOT NULL DEFAULT 0,
                conclusions_derived INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                provider TEXT,
                model TEXT,
                error TEXT,
                created_at_ms INTEGER NOT NULL
            )",
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
                created_at_ms INTEGER NOT NULL,
                last_decayed_at_ms INTEGER,
                pinned INTEGER NOT NULL DEFAULT 0
            )",
            "CREATE TABLE user_context_conclusion_evidence (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conclusion_id INTEGER NOT NULL REFERENCES user_context_conclusions(id) ON DELETE CASCADE,
                activity_id INTEGER NOT NULL REFERENCES user_context_activities(id) ON DELETE CASCADE,
                stance TEXT NOT NULL DEFAULT 'support',
                created_at_ms INTEGER NOT NULL,
                UNIQUE (conclusion_id, activity_id)
            )",
            "CREATE TABLE user_context_confidence_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conclusion_id INTEGER NOT NULL REFERENCES user_context_conclusions(id) ON DELETE CASCADE,
                confidence REAL NOT NULL,
                snapshot_at_ms INTEGER NOT NULL
            )",
            "CREATE TABLE user_context_dismissals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                subject TEXT NOT NULL,
                statement TEXT NOT NULL,
                evidence_fingerprint TEXT NOT NULL,
                evidence_activity_count INTEGER NOT NULL DEFAULT 0,
                dismissed_at_ms INTEGER NOT NULL
            )",
            // Minimal raw-capture tables so `earliest_capture_at_ms` (#98) can be
            // tested. Only the timestamp columns it reads are modeled.
            "CREATE TABLE frames (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                captured_at TEXT NOT NULL
            )",
            "CREATE TABLE audio_segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                started_at TEXT NOT NULL
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

    #[test]
    fn confidence_history_snapshots_round_trip_ascending() {
        block_on(async {
        let store = test_store().await;
        let id = store
            .upsert_conclusion(draft("Rust", "Learning Rust", 0.5))
            .await
            .expect("upsert");

        // Insert out of order; list_confidence_history returns ascending time.
        store.insert_confidence_snapshot(id, 0.50, 3_000).await.expect("snap 3");
        store.insert_confidence_snapshot(id, 0.40, 1_000).await.expect("snap 1");
        store.insert_confidence_snapshot(id, 0.45, 2_000).await.expect("snap 2");

        let history = store.list_confidence_history(id).await.expect("history");
        let times: Vec<i64> = history.iter().map(|s| s.snapshot_at_ms).collect();
        assert_eq!(times, vec![1_000, 2_000, 3_000], "ascending snapshot_at_ms");
        assert_eq!(history[0].confidence, 0.40);
        });
    }

    #[test]
    fn prune_confidence_history_keeps_newest_n_per_conclusion() {
        block_on(async {
        let store = test_store().await;
        let a = store
            .upsert_conclusion(draft("Rust", "Learning Rust", 0.5))
            .await
            .expect("a");
        let b = store
            .upsert_conclusion(draft("Apple", "Likes Apple", 0.5))
            .await
            .expect("b");

        for t in [1_000, 2_000, 3_000, 4_000, 5_000] {
            store.insert_confidence_snapshot(a, 0.5, t).await.expect("snap a");
        }
        store.insert_confidence_snapshot(b, 0.5, 1_000).await.expect("snap b");

        // Keep newest 2 per conclusion: A loses 3 of its 5, B keeps its single.
        let deleted = store.prune_confidence_history(2).await.expect("prune");
        assert_eq!(deleted, 3, "three of A's oldest snapshots removed");

        let a_history = store.list_confidence_history(a).await.expect("a history");
        let a_times: Vec<i64> = a_history.iter().map(|s| s.snapshot_at_ms).collect();
        assert_eq!(a_times, vec![4_000, 5_000], "newest two kept");
        // B is untouched (it had fewer than the cap).
        assert_eq!(store.list_confidence_history(b).await.expect("b history").len(), 1);
        });
    }

    #[test]
    fn update_conclusion_confidence_persists_value_and_status() {
        block_on(async {
        let store = test_store().await;
        let id = store
            .upsert_conclusion(draft("Vim", "Used Vim", 0.6))
            .await
            .expect("upsert");

        store
            .update_conclusion_confidence(id, 0.10, ConclusionStatus::Faded, 9_999)
            .await
            .expect("update");

        // A faded Conclusion is excluded from the visible-only list but appears
        // with include_faded, carrying the decayed confidence.
        assert!(store.list_conclusions(false).await.expect("visible").is_empty());
        let faded = store.list_conclusions(true).await.expect("with faded");
        assert_eq!(faded.len(), 1);
        assert_eq!(faded[0].confidence, 0.10);
        assert_eq!(faded[0].status, ConclusionStatus::Faded);
        });
    }

    #[test]
    fn list_decayable_conclusions_excludes_dismissed_only() {
        block_on(async {
        let store = test_store().await;
        let visible = store
            .upsert_conclusion(draft("Rust", "Learning Rust", 0.8))
            .await
            .expect("visible");
        let faded = store
            .upsert_conclusion(draft("Vim", "Used Vim", 0.1))
            .await
            .expect("faded");
        let dismissed = store
            .upsert_conclusion(draft("Coffee", "Hates coffee", 0.6))
            .await
            .expect("dismissed");
        store
            .set_conclusion_status(faded, ConclusionStatus::Faded)
            .await
            .expect("set faded");
        store
            .set_conclusion_status(dismissed, ConclusionStatus::Dismissed)
            .await
            .expect("set dismissed");

        let decayable = store.list_decayable_conclusions().await.expect("decayable");
        let ids: Vec<i64> = decayable.iter().map(|c| c.id).collect();
        assert!(ids.contains(&visible), "visible is decayable");
        assert!(ids.contains(&faded), "faded is decayable (history still snapshotted)");
        assert!(!ids.contains(&dismissed), "dismissed is not decayable");
        });
    }

    #[test]
    fn evidence_fingerprint_is_deterministic_and_sorted_distinct() {
        // Order- and duplicate-independent: the same id set yields the same string.
        assert_eq!(evidence_fingerprint(&[3, 1, 2]), "1,2,3");
        assert_eq!(evidence_fingerprint(&[2, 1, 3]), "1,2,3");
        assert_eq!(evidence_fingerprint(&[3, 1, 2, 1, 3]), "1,2,3");
        // Empty set → empty string.
        assert_eq!(evidence_fingerprint(&[]), "");
        // A single id.
        assert_eq!(evidence_fingerprint(&[42]), "42");
    }

    #[test]
    fn dismiss_records_state_and_removes_conclusion() {
        block_on(async {
        let store = test_store().await;
        let a1 = seed_activity(&store, "Read Apple news", 100).await;
        let a2 = seed_activity(&store, "Watched Apple keynote", 200).await;
        let contradict = seed_activity(&store, "Bought a Pixel", 300).await;
        let id = store
            .upsert_conclusion(draft("Apple", "Interested in Apple", 0.6))
            .await
            .expect("upsert");
        store
            .replace_conclusion_evidence(
                id,
                vec![
                    NewConclusionEvidence { activity_id: a1, stance: EvidenceStance::Support },
                    NewConclusionEvidence { activity_id: a2, stance: EvidenceStance::Support },
                    NewConclusionEvidence {
                        activity_id: contradict,
                        stance: EvidenceStance::Contradict,
                    },
                ],
            )
            .await
            .expect("evidence");

        store.dismiss_conclusion(id).await.expect("dismiss");

        // The Conclusion is gone (Dismiss removes it).
        assert!(store.get_conclusion(id).await.expect("get").is_none());
        assert_eq!(store.count_conclusions().await.expect("count"), 0);

        // The Dismissal State persists, with the support count (2, not 3) and a
        // fingerprint of ALL distinct evidence ids (support + contradict).
        let dismissals = store.list_dismissals().await.expect("dismissals");
        assert_eq!(dismissals.len(), 1);
        assert_eq!(dismissals[0].subject, "Apple");
        assert_eq!(dismissals[0].statement, "Interested in Apple");
        assert_eq!(dismissals[0].evidence_activity_count, 2, "support-stance count");
        assert_eq!(
            dismissals[0].evidence_fingerprint,
            evidence_fingerprint(&[a1, a2, contradict])
        );

        // Subject-scoped listing finds it case-insensitively.
        let by_subject = store
            .list_dismissals_for_subject("apple")
            .await
            .expect("by subject");
        assert_eq!(by_subject.len(), 1);
        // A different subject sees nothing.
        assert!(store
            .list_dismissals_for_subject("Rust")
            .await
            .expect("other subject")
            .is_empty());
        });
    }

    #[test]
    fn dismiss_missing_conclusion_records_nothing() {
        block_on(async {
        let store = test_store().await;
        store.dismiss_conclusion(9999).await.expect("dismiss noop");
        assert!(store.list_dismissals().await.expect("dismissals").is_empty());
        });
    }

    #[test]
    fn set_pinned_excludes_from_list_decayable_conclusions() {
        block_on(async {
        let store = test_store().await;
        let pinned = store
            .upsert_conclusion(draft("Rust", "Learning Rust", 0.8))
            .await
            .expect("pinned");
        let unpinned = store
            .upsert_conclusion(draft("Vim", "Used Vim", 0.6))
            .await
            .expect("unpinned");

        store.set_pinned(pinned, true).await.expect("pin");

        // The pinned row maps as pinned and is dropped from the decayable set;
        // the unpinned row stays decayable.
        let fetched = store.get_conclusion(pinned).await.expect("get").expect("present");
        assert!(fetched.pinned, "pinned column is read back as true");

        let decayable = store.list_decayable_conclusions().await.expect("decayable");
        let ids: Vec<i64> = decayable.iter().map(|c| c.id).collect();
        assert!(!ids.contains(&pinned), "pinned is exempt from decay");
        assert!(ids.contains(&unpinned), "unpinned stays decayable");

        // Unpinning restores decayability.
        store.set_pinned(pinned, false).await.expect("unpin");
        let after = store.list_decayable_conclusions().await.expect("decayable");
        assert!(after.iter().any(|c| c.id == pinned), "unpinned is decayable again");
        });
    }

    /// Seed an Activity with a SINGLE evidence row of the given subject, so the
    /// cascade test can target it precisely by (subject_type, subject_id).
    async fn seed_activity_with_subject(
        store: &UserContextStore,
        title: &str,
        started_at_ms: i64,
        subject_type: &str,
        subject_id: i64,
    ) -> i64 {
        store
            .insert_activity_with_evidence(NewActivity {
                title: title.to_string(),
                summary: format!("{title} summary"),
                category: None,
                started_at_ms,
                ended_at_ms: started_at_ms + 1,
                derivation_run_id: None,
                evidence: vec![NewActivityEvidence {
                    subject_type: subject_type.to_string(),
                    subject_id,
                    captured_at_ms: Some(started_at_ms),
                }],
            })
            .await
            .expect("insert activity")
    }

    #[test]
    fn delete_derived_for_capture_subjects_drops_ungrounded_keeps_grounded() {
        block_on(async {
        let store = test_store().await;

        // Activity A is grounded in frame 10 (which will be deleted).
        // Activity B is grounded in frame 20 (which survives).
        let activity_a = seed_activity_with_subject(&store, "Worked on the spec", 100, "frame", 10).await;
        let activity_b = seed_activity_with_subject(&store, "Reviewed a PR", 200, "frame", 20).await;
        // Activity C is grounded in audio segment 30 (which will be deleted).
        let activity_c = seed_activity_with_subject(&store, "Call about the spec", 300, "audio_segment", 30).await;

        // Conclusion 1: grounded ONLY by A (lost entirely when A goes) -> dropped.
        let only_a = store
            .upsert_conclusion(draft("Spec", "Cares about the spec", 0.7))
            .await
            .expect("only_a");
        store
            .replace_conclusion_evidence(
                only_a,
                vec![NewConclusionEvidence { activity_id: activity_a, stance: EvidenceStance::Support }],
            )
            .await
            .expect("evidence only_a");

        // Conclusion 2: grounded by BOTH A and B; A goes but B survives -> stays.
        let a_and_b = store
            .upsert_conclusion(draft("Work", "Active on work", 0.6))
            .await
            .expect("a_and_b");
        store
            .replace_conclusion_evidence(
                a_and_b,
                vec![
                    NewConclusionEvidence { activity_id: activity_a, stance: EvidenceStance::Support },
                    NewConclusionEvidence { activity_id: activity_b, stance: EvidenceStance::Support },
                ],
            )
            .await
            .expect("evidence a_and_b");

        // Conclusion 3: grounded ONLY by C (audio); C goes -> dropped.
        let only_c = store
            .upsert_conclusion(draft("Calls", "On calls", 0.5))
            .await
            .expect("only_c");
        store
            .replace_conclusion_evidence(
                only_c,
                vec![NewConclusionEvidence { activity_id: activity_c, stance: EvidenceStance::Support }],
            )
            .await
            .expect("evidence only_c");

        // A dismissal row keyed by subject/statement: must survive the cascade.
        let dismissed = store
            .upsert_conclusion(draft("Vim", "Uses Vim", 0.4))
            .await
            .expect("dismissed");
        store.dismiss_conclusion(dismissed).await.expect("dismiss");
        assert_eq!(store.list_dismissals().await.expect("dismissals").len(), 1);

        // Cascade: frame 10 + audio segment 30 were deleted (frame 20 survives).
        let summary = store
            .delete_derived_for_capture_subjects(&[10], &[30])
            .await
            .expect("cascade");

        // Activities A and C dropped; B survives.
        assert_eq!(summary.deleted_activities, 2, "A (frame 10) and C (audio 30)");
        assert!(store.list_recent_activities(100, 0).await.expect("activities")
            .iter().all(|a| a.id != activity_a && a.id != activity_c));
        assert!(store.list_recent_activities(100, 0).await.expect("activities")
            .iter().any(|a| a.id == activity_b), "B (frame 20) survives");

        // Conclusion 1 (only A) and 3 (only C) dropped; Conclusion 2 (A+B) stays
        // because B's evidence link survives.
        assert_eq!(summary.deleted_conclusions, 2, "only_a and only_c dropped");
        assert!(store.get_conclusion(only_a).await.expect("get").is_none());
        assert!(store.get_conclusion(only_c).await.expect("get").is_none());
        let surviving = store.get_conclusion(a_and_b).await.expect("get").expect("a_and_b stays");
        assert_eq!(surviving.evidence.len(), 1, "only B's evidence link remains");
        assert_eq!(surviving.evidence[0].activity_id, activity_b);

        // Dismissal State is untouched by the capture cascade.
        let dismissals = store.list_dismissals().await.expect("dismissals after");
        assert_eq!(dismissals.len(), 1, "dismissal survives capture deletion");
        assert_eq!(dismissals[0].statement, "Uses Vim");
        });
    }

    #[test]
    fn delete_derived_for_capture_subjects_is_noop_for_empty_ids() {
        block_on(async {
        let store = test_store().await;
        let activity = seed_activity_with_subject(&store, "Kept", 100, "frame", 10).await;
        let summary = store
            .delete_derived_for_capture_subjects(&[], &[])
            .await
            .expect("noop cascade");
        assert_eq!(summary.deleted_activities, 0);
        assert_eq!(summary.deleted_conclusions, 0);
        assert!(store.list_recent_activities(100, 0).await.expect("activities")
            .iter().any(|a| a.id == activity), "nothing deleted on empty ids");
        });
    }

    /// #98 backfill-position SQL: `oldest_derivation_run_window_start` returns
    /// the MIN windowed-run start over `activity`/`backfill` runs only (NULL-bound
    /// `conclusion`/`confidence` runs are ignored), and `earliest_capture_at_ms`
    /// takes the MIN across `frames.captured_at` / `audio_segments.started_at`,
    /// RFC3339 → millis.
    #[test]
    fn backfill_position_helpers_compute_floor_and_oldest_covered() {
        block_on(async {
        let store = test_store().await;

        // No runs / no captures yet → both None.
        assert_eq!(
            store.oldest_derivation_run_window_start().await.expect("oldest"),
            None
        );
        assert_eq!(
            store.earliest_capture_at_ms().await.expect("earliest"),
            None
        );

        // Windowed runs: an activity window [5000, 6000] and an older backfill
        // window [2000, 3000] → oldest start is 2000.
        for (kind, start, end) in [("activity", 5_000, 6_000), ("backfill", 2_000, 3_000)] {
            store
                .insert_derivation_run(NewDerivationRun {
                    kind: kind.to_string(),
                    window_start_ms: Some(start),
                    window_end_ms: Some(end),
                    status: "completed".to_string(),
                    activities_derived: 0,
                    conclusions_derived: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: None,
                    model: None,
                    error: None,
                })
                .await
                .expect("windowed run");
        }
        // A NULL-bound conclusion run with an even smaller-looking window must NOT
        // pull the floor down: it is excluded by kind + IS NOT NULL.
        store
            .insert_derivation_run(NewDerivationRun {
                kind: "conclusion".to_string(),
                window_start_ms: None,
                window_end_ms: None,
                status: "completed".to_string(),
                activities_derived: 0,
                conclusions_derived: 1,
                input_tokens: 0,
                output_tokens: 0,
                provider: None,
                model: None,
                error: None,
            })
            .await
            .expect("conclusion run");

        assert_eq!(
            store.oldest_derivation_run_window_start().await.expect("oldest"),
            Some(2_000),
            "MIN over activity/backfill window starts"
        );

        // Captures: a frame at 2020-01-01T00:00:10Z and an earlier audio segment
        // at 2020-01-01T00:00:05Z → MIN is the audio start.
        sqlx::query("INSERT INTO frames (captured_at) VALUES (?1)")
            .bind("2020-01-01T00:00:10Z")
            .execute(store.pool())
            .await
            .expect("frame");
        sqlx::query("INSERT INTO audio_segments (started_at) VALUES (?1)")
            .bind("2020-01-01T00:00:05Z")
            .execute(store.pool())
            .await
            .expect("audio segment");

        let expected_ms = rfc3339_text_to_ms("2020-01-01T00:00:05Z").expect("parse");
        assert_eq!(
            store.earliest_capture_at_ms().await.expect("earliest"),
            Some(expected_ms),
            "MIN across frames/audio_segments, RFC3339 → millis"
        );
        });
    }

    #[test]
    fn wipe_all_empties_every_user_context_table() {
        block_on(async {
        let store = test_store().await;

        // Populate every table: an Activity (+ evidence), a Conclusion
        // (+ evidence + confidence history), a derivation run, and a dismissal.
        let activity = seed_activity_with_subject(&store, "Did a thing", 100, "frame", 10).await;
        let conclusion = store
            .upsert_conclusion(draft("Topic", "Engaged with topic", 0.8))
            .await
            .expect("conclusion");
        store
            .replace_conclusion_evidence(
                conclusion,
                vec![NewConclusionEvidence { activity_id: activity, stance: EvidenceStance::Support }],
            )
            .await
            .expect("evidence");
        store.insert_confidence_snapshot(conclusion, 0.8, 1_000).await.expect("snapshot");
        store
            .insert_derivation_run(NewDerivationRun {
                kind: "activity".to_string(),
                window_start_ms: Some(0),
                window_end_ms: Some(1),
                status: "completed".to_string(),
                activities_derived: 1,
                conclusions_derived: 0,
                input_tokens: 10,
                output_tokens: 5,
                provider: Some("test".to_string()),
                model: Some("test".to_string()),
                error: None,
            })
            .await
            .expect("derivation run");
        let to_dismiss = store
            .upsert_conclusion(draft("Vim", "Uses Vim", 0.4))
            .await
            .expect("to dismiss");
        store.dismiss_conclusion(to_dismiss).await.expect("dismiss");

        // Sanity: everything present before the wipe.
        assert!(store.count_activities().await.expect("count") > 0);
        assert!(store.count_conclusions().await.expect("count") > 0);
        assert!(!store.list_dismissals().await.expect("dismissals").is_empty());

        store.wipe_all().await.expect("wipe");

        // Every table is empty.
        assert_eq!(store.count_activities().await.expect("count"), 0);
        assert_eq!(store.count_conclusions().await.expect("count"), 0);
        assert!(store.list_dismissals().await.expect("dismissals").is_empty());
        for table in [
            "user_context_activity_evidence",
            "user_context_conclusion_evidence",
            "user_context_confidence_history",
            "user_context_activities",
            "user_context_conclusions",
            "user_context_dismissals",
            "user_context_derivation_runs",
        ] {
            let count: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                    .fetch_one(store.pool())
                    .await
                    .expect("count table");
            assert_eq!(count, 0, "{table} should be empty after wipe");
        }
        });
    }

    /// Regression for ADR 0029: time-based **Retention Policy** aging of raw
    /// media must NOT cascade into derived data. This asserts the structural
    /// guarantee directly — the `capture_retention` delete path never names a
    /// `user_context_*` table — so aging a frame out leaves the Activity derived
    /// from it intact (only Delete Recent Capture cascades).
    #[test]
    fn retention_cleanup_source_never_touches_user_context_tables() {
        let retention_src = include_str!("../capture_retention.rs");
        assert!(
            !retention_src.contains("user_context"),
            "capture_retention.rs must not reference any user_context_* table; \
             Retention Policy aging must not cascade into derived data (ADR 0029)"
        );
    }
}
