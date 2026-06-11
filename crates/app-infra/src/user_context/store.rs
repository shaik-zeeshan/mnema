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
    Activity, ActivityCategory, ActivityEvidenceRef, AuthoredContext, Conclusion,
    ConclusionEvidenceRef, ConclusionStatus, ConfidenceSnapshot, DismissalState, EvidenceStance,
    FocusLevel, UserContextTokenUsage,
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
    /// Engine-assigned Activity Category (#105). User corrections are recorded
    /// separately via [`UserContextStore::correct_activity`] and win on read.
    pub category: Option<ActivityCategory>,
    /// Engine-assigned Focus Classification (#105). User corrections are
    /// recorded separately and win on read.
    pub focus: Option<FocusLevel>,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub derivation_run_id: Option<i64>,
    pub evidence: Vec<NewActivityEvidence>,
}

/// A user **correction** of an Activity's Category and/or Focus (#108), as read
/// back for the derivation feedback loop. Carries the *effective* corrected
/// values (the engine label is irrelevant once corrected) plus the Activity's
/// title/summary so the next derivation prompt can bias the engine away from
/// regenerating the corrected-away label for similar activities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityCorrection {
    pub activity_id: i64,
    pub title: String,
    pub summary: String,
    /// The user's corrected Category (may be `None`: corrected to "unset").
    pub corrected_category: Option<ActivityCategory>,
    /// The user's corrected Focus (may be `None`: corrected to "unset").
    pub corrected_focus: Option<FocusLevel>,
    pub corrected_at_ms: i64,
}

/// One raw-capture evidence reference for a [`NewActivity`]. `subject_type` is
/// `"frame"` | `"audio_segment"` (mirrors `processing_jobs` subject types).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewActivityEvidence {
    pub subject_type: String,
    pub subject_id: i64,
    pub captured_at_ms: Option<i64>,
}

/// Per-gate withheld counters for one Conclusion-distillation pass: how many
/// engine drafts each deterministic persist gate dropped, in gate order. Always
/// zero for non-`'conclusion'` run kinds. These make "distillation produced
/// nothing" diagnosable — without them a pass whose drafts were all withheld by
/// policy is indistinguishable from one the engine returned empty.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DistillationGateDrops {
    /// Drafts with no resolvable supporting Activity reference.
    pub ungrounded: i64,
    /// Sensitive Category Guardrail hard post-filter drops (#96).
    pub guardrail_suppressed: i64,
    /// Drafts below the formation bar's supporting-evidence minimum (#95).
    pub below_formation_bar: i64,
    /// Dismissed Conclusions that did not clear the resurface bar (#99).
    pub resurface_blocked: i64,
}

impl DistillationGateDrops {
    pub fn total(&self) -> i64 {
        self.ungrounded
            + self.guardrail_suppressed
            + self.below_formation_bar
            + self.resurface_blocked
    }
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
    /// Per-gate withheld counts; meaningful only on `'conclusion'` runs.
    pub gate_drops: DistillationGateDrops,
}

/// A `failed` derivation window eligible for a retry (issue #113): a
/// `[window_start_ms, window_end_ms]` span whose every windowed run failed —
/// no later `completed`/`skipped` run ever covered the same span — so the
/// Activity history has a hole there. Returned by
/// [`UserContextStore::failed_windows_eligible_for_retry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedDerivationWindow {
    /// The kind of the most recent failed run over this span (`'activity'` or
    /// `'backfill'`); a retry records its run under the same kind.
    pub kind: String,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    /// How many failed runs cover exactly this span (the retry-cap counter).
    pub failure_count: i64,
    /// `created_at_ms` of the newest failed run (the backoff anchor).
    pub last_failed_at_ms: i64,
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
/// **Activity** rows, how many now-below-the-formation-bar **Conclusion** rows,
/// and how many now-stale **Digest** rows were dropped. Used for the warning
/// log + UI refresh; not persisted.
#[derive(Debug, Clone, Default)]
pub struct UserContextCascadeSummary {
    pub deleted_activities: i64,
    pub deleted_conclusions: i64,
    pub deleted_digests: i64,
}

/// A stored **Digest** (migration 0029): the Insights Overview's engine-written
/// narrative lede for one `(range_kind, range_start_ms)` range, plus the
/// [`digest_input_fingerprint`] of the Activities it was derived from so the
/// Tauri layer can detect staleness and regenerate lazily.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDigest {
    /// `'day'` | `'week'` | `'month'`.
    pub range_kind: String,
    pub range_start_ms: i64,
    /// Exclusive: the digest covers `[range_start_ms, range_end_ms)`.
    pub range_end_ms: i64,
    pub narrative: String,
    /// Short generated title rendered above the narrative; `None` on rows
    /// written before the headline existed (migration 0030).
    pub headline: Option<String>,
    pub input_fingerprint: String,
    pub generated_at_ms: i64,
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
                (title, summary, category, focus, started_at_ms, ended_at_ms, derivation_run_id, created_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(&draft.title)
        .bind(&draft.summary)
        .bind(draft.category.map(category_to_str))
        .bind(draft.focus.map(focus_to_str))
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
            "SELECT id, title, summary, category, focus, corrected_category, category_corrected, \
                    corrected_focus, focus_corrected, started_at_ms, ended_at_ms, created_at_ms \
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

    /// Every Activity overlapping the half-open `[range_start_ms, range_end_ms)`
    /// window, chronological (oldest first) — the **Digest** input set. The
    /// overlap predicate matches the digest staleness purge in
    /// [`Self::delete_derived_for_capture_subjects`]:
    /// `started_at_ms < range_end AND ended_at_ms >= range_start`.
    ///
    /// Evidence refs are NOT hydrated (each row's `evidence` is empty): both
    /// Digest consumers — [`digest_input_fingerprint`] and the narrative
    /// prompt — read only the Activity's own fields, and a month range can
    /// hold many Activities (hydration is one extra query per row).
    pub async fn list_activities_in_range(
        &self,
        range_start_ms: i64,
        range_end_ms: i64,
    ) -> Result<Vec<Activity>> {
        let rows = sqlx::query(
            "SELECT id, title, summary, category, focus, corrected_category, category_corrected, \
                    corrected_focus, focus_corrected, started_at_ms, ended_at_ms, created_at_ms \
             FROM user_context_activities \
             WHERE started_at_ms < ?2 AND ended_at_ms >= ?1 \
             ORDER BY started_at_ms ASC, id ASC",
        )
        .bind(range_start_ms)
        .bind(range_end_ms)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(map_activity).collect())
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

    // --- #108: Category / Focus corrections -------------------------------

    /// Record a user **correction** of an Activity's Category and/or Focus
    /// (#108). Each `Option<Option<_>>` argument is "leave unchanged" (`None`)
    /// vs "set this correction" (`Some(value)`, where `value` may itself be
    /// `None` = correct to "unset"). When a correction is set, its `*_corrected`
    /// flag is raised so the corrected value WINS over the engine label on read,
    /// even when the corrected value is NULL. Stamps `corrected_at_ms` whenever
    /// any field is corrected. A no-op (no timestamp bump) when both args are
    /// `None` or `id` names no Activity.
    pub async fn correct_activity(
        &self,
        id: i64,
        category: Option<Option<ActivityCategory>>,
        focus: Option<Option<FocusLevel>>,
    ) -> Result<()> {
        // Nothing to change.
        if category.is_none() && focus.is_none() {
            return Ok(());
        }
        let now = now_ms();
        let mut builder = QueryBuilder::<Sqlite>::new("UPDATE user_context_activities SET ");
        let mut separated = builder.separated(", ");
        if let Some(corrected) = category {
            separated.push("corrected_category = ");
            separated.push_bind_unseparated(corrected.map(category_to_str).map(str::to_string));
            separated.push("category_corrected = 1");
        }
        if let Some(corrected) = focus {
            separated.push("corrected_focus = ");
            separated.push_bind_unseparated(corrected.map(focus_to_str).map(str::to_string));
            separated.push("focus_corrected = 1");
        }
        separated.push("corrected_at_ms = ");
        separated.push_bind_unseparated(now);
        builder.push(" WHERE id = ");
        builder.push_bind(id);
        builder.build().execute(&self.pool).await?;
        Ok(())
    }

    /// Every Activity the user has corrected (#108), newest correction first.
    /// Fed to the derivation pass so the engine is biased away from regenerating
    /// a corrected-away Category/Focus for similar activities. Carries the
    /// *effective* corrected values (the engine label is irrelevant once
    /// corrected). Capped at `limit` (the most recent corrections matter most).
    pub async fn list_activity_corrections(&self, limit: i64) -> Result<Vec<ActivityCorrection>> {
        let rows = sqlx::query(
            "SELECT id, title, summary, corrected_category, category_corrected, \
                    corrected_focus, focus_corrected, corrected_at_ms \
             FROM user_context_activities \
             WHERE category_corrected = 1 OR focus_corrected = 1 \
             ORDER BY corrected_at_ms DESC, id DESC \
             LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let corrected_category = if row.get::<i64, _>("category_corrected") != 0 {
                    category_from_str(
                        row.get::<Option<String>, _>("corrected_category").as_deref(),
                    )
                } else {
                    None
                };
                let corrected_focus = if row.get::<i64, _>("focus_corrected") != 0 {
                    focus_from_str(row.get::<Option<String>, _>("corrected_focus").as_deref())
                } else {
                    None
                };
                ActivityCorrection {
                    activity_id: row.get("id"),
                    title: row.get("title"),
                    summary: row.get("summary"),
                    corrected_category,
                    corrected_focus,
                    corrected_at_ms: row.get::<Option<i64>, _>("corrected_at_ms").unwrap_or(0),
                }
            })
            .collect())
    }

    // --- #93: Derivation runs ---------------------------------------------

    /// Inserts a derivation-run ledger row, returning its id.
    pub async fn insert_derivation_run(&self, run: NewDerivationRun) -> Result<i64> {
        let created_at_ms = now_ms();
        let id = sqlx::query(
            "INSERT INTO user_context_derivation_runs \
                (kind, window_start_ms, window_end_ms, status, activities_derived, \
                 conclusions_derived, input_tokens, output_tokens, provider, model, error, \
                 ungrounded, guardrail_suppressed, below_formation_bar, resurface_blocked, \
                 created_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
        .bind(run.gate_drops.ungrounded)
        .bind(run.gate_drops.guardrail_suppressed)
        .bind(run.gate_drops.below_formation_bar)
        .bind(run.gate_drops.resurface_blocked)
        .bind(created_at_ms)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    /// The most recent **completed** Conclusion-distillation run (kind
    /// `'conclusion'`): when it ran, how many Conclusions it upserted, and how
    /// many drafts each persist gate withheld. Powers the settings readout's
    /// "why is my dossier thin?" line. `None` until a distillation completes.
    pub async fn latest_distillation_summary(
        &self,
    ) -> Result<Option<(i64, i64, DistillationGateDrops)>> {
        let row = sqlx::query(
            "SELECT created_at_ms, conclusions_derived, ungrounded, guardrail_suppressed, \
                    below_formation_bar, resurface_blocked \
             FROM user_context_derivation_runs \
             WHERE kind = 'conclusion' AND status = 'completed' \
             ORDER BY created_at_ms DESC, id DESC \
             LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| {
            (
                row.get::<i64, _>("created_at_ms"),
                row.get::<i64, _>("conclusions_derived"),
                DistillationGateDrops {
                    ungrounded: row.get("ungrounded"),
                    guardrail_suppressed: row.get("guardrail_suppressed"),
                    below_formation_bar: row.get("below_formation_bar"),
                    resurface_blocked: row.get("resurface_blocked"),
                },
            )
        }))
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

    /// `failed` derivation windows that still have a hole in Activity coverage
    /// and are eligible for a retry (issue #113): windowed (`'activity'` /
    /// `'backfill'`) spans where every run failed — never covered by a
    /// `completed`/`skipped` run over the same exact span — with fewer than
    /// `max_failures` failed runs (the crash-loop backstop) whose newest
    /// failure is at or before `last_failed_at_or_before_ms` (the wall-clock
    /// backoff). Newest-first, matching the History Backfill policy, capped at
    /// `limit`.
    ///
    /// Exact-span matching is sound because both the forward beat and backfill
    /// step the cursor in whole windows: a retried span is re-derived with the
    /// same bounds, so its success/skip run extinguishes the hole.
    pub async fn failed_windows_eligible_for_retry(
        &self,
        max_failures: i64,
        last_failed_at_or_before_ms: i64,
        limit: i64,
    ) -> Result<Vec<FailedDerivationWindow>> {
        let rows = sqlx::query(
            "SELECT f.window_start_ms AS window_start_ms, \
                    f.window_end_ms AS window_end_ms, \
                    COUNT(*) AS failure_count, \
                    MAX(f.created_at_ms) AS last_failed_at_ms, \
                    (SELECT k.kind FROM user_context_derivation_runs k \
                      WHERE k.window_start_ms = f.window_start_ms \
                        AND k.window_end_ms = f.window_end_ms \
                        AND k.status = 'failed' \
                        AND k.kind IN ('activity', 'backfill') \
                      ORDER BY k.created_at_ms DESC, k.id DESC \
                      LIMIT 1) AS kind \
             FROM user_context_derivation_runs f \
             WHERE f.kind IN ('activity', 'backfill') \
               AND f.status = 'failed' \
               AND f.window_start_ms IS NOT NULL \
               AND f.window_end_ms IS NOT NULL \
               AND NOT EXISTS (\
                   SELECT 1 FROM user_context_derivation_runs s \
                   WHERE s.window_start_ms = f.window_start_ms \
                     AND s.window_end_ms = f.window_end_ms \
                     AND s.status IN ('completed', 'skipped')\
               ) \
             GROUP BY f.window_start_ms, f.window_end_ms \
             HAVING COUNT(*) < ?1 AND MAX(f.created_at_ms) <= ?2 \
             ORDER BY f.window_end_ms DESC \
             LIMIT ?3",
        )
        .bind(max_failures)
        .bind(last_failed_at_or_before_ms)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| FailedDerivationWindow {
                kind: row.get("kind"),
                window_start_ms: row.get("window_start_ms"),
                window_end_ms: row.get("window_end_ms"),
                failure_count: row.get("failure_count"),
                last_failed_at_ms: row.get("last_failed_at_ms"),
            })
            .collect())
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
            "SELECT id, title, summary, category, focus, corrected_category, category_corrected, \
                    corrected_focus, focus_corrected, started_at_ms, ended_at_ms, created_at_ms \
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

    // --- #107: User-authored Context --------------------------------------

    /// Insert a standing **user-authored Context** statement, returning its id.
    /// `text` is stored verbatim; `topic` is an optional grouping handle.
    /// Authored Context is user-asserted (not derived), carries no confidence,
    /// and never decays.
    pub async fn add_authored_context(
        &self,
        text: &str,
        topic: Option<&str>,
        now_ms: i64,
    ) -> Result<i64> {
        let id = sqlx::query(
            "INSERT INTO user_context_authored (text, topic, created_at_ms, updated_at_ms) \
             VALUES (?1, ?2, ?3, ?3)",
        )
        .bind(text)
        .bind(topic)
        .bind(now_ms)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();
        Ok(id)
    }

    /// Update a user-authored Context statement's `text`/`topic`, bumping
    /// `updated_at_ms`. A no-op when `id` names no row.
    pub async fn update_authored_context(
        &self,
        id: i64,
        text: &str,
        topic: Option<&str>,
        now_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE user_context_authored \
             SET text = ?2, topic = ?3, updated_at_ms = ?4 \
             WHERE id = ?1",
        )
        .bind(id)
        .bind(text)
        .bind(topic)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete a user-authored Context statement. A no-op when `id` is absent.
    pub async fn delete_authored_context(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM user_context_authored WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List every user-authored Context statement, newest first (by
    /// `created_at_ms`). Used by both the settings surface and the derivation
    /// pass (which feeds them to the engine as standing context).
    pub async fn list_authored_context(&self) -> Result<Vec<AuthoredContext>> {
        let rows = sqlx::query(
            "SELECT id, text, topic, created_at_ms, updated_at_ms \
             FROM user_context_authored \
             ORDER BY created_at_ms DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AuthoredContext {
                id: row.get("id"),
                text: row.get("text"),
                topic: row.get("topic"),
                created_at_ms: row.get("created_at_ms"),
                updated_at_ms: row.get("updated_at_ms"),
            })
            .collect())
    }

    // --- Digests: the Insights Overview narrative lede ----------------------

    /// The stored **Digest** for one `(range_kind, range_start_ms)` range, or
    /// `None` when nothing has been generated for it yet. The Tauri layer
    /// compares the stored `input_fingerprint` against a fresh
    /// [`digest_input_fingerprint`] to decide whether to regenerate.
    pub async fn get_digest(
        &self,
        range_kind: &str,
        range_start_ms: i64,
    ) -> Result<Option<StoredDigest>> {
        let row = sqlx::query(
            "SELECT range_kind, range_start_ms, range_end_ms, narrative, headline, \
                    input_fingerprint, generated_at_ms \
             FROM user_context_digests \
             WHERE range_kind = ?1 AND range_start_ms = ?2",
        )
        .bind(range_kind)
        .bind(range_start_ms)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| StoredDigest {
            range_kind: row.get("range_kind"),
            range_start_ms: row.get("range_start_ms"),
            range_end_ms: row.get("range_end_ms"),
            narrative: row.get("narrative"),
            headline: row.get("headline"),
            input_fingerprint: row.get("input_fingerprint"),
            generated_at_ms: row.get("generated_at_ms"),
        }))
    }

    /// Every stored DAY-kind **Digest** whose `[range_start_ms, range_end_ms)`
    /// half-open span overlaps the given `[range_start_ms, range_end_ms)`
    /// window, in chronological order (`range_start_ms ASC`). Two half-open
    /// ranges overlap exactly when each starts before the other ends, hence
    /// `range_start_ms < ?2 AND range_end_ms > ?1`.
    ///
    /// A wider (week/month) digest reuses these cached day-digest narratives as
    /// low-detail "rollup" lines — the hybrid path that avoids re-deriving each
    /// day from raw Activities.
    pub async fn list_day_digests_in_range(
        &self,
        range_start_ms: i64,
        range_end_ms: i64,
    ) -> Result<Vec<StoredDigest>> {
        let rows = sqlx::query(
            "SELECT range_kind, range_start_ms, range_end_ms, narrative, headline, \
                    input_fingerprint, generated_at_ms \
             FROM user_context_digests \
             WHERE range_kind = 'day' AND range_start_ms < ?2 AND range_end_ms > ?1 \
             ORDER BY range_start_ms ASC",
        )
        .bind(range_start_ms)
        .bind(range_end_ms)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| StoredDigest {
                range_kind: row.get("range_kind"),
                range_start_ms: row.get("range_start_ms"),
                range_end_ms: row.get("range_end_ms"),
                narrative: row.get("narrative"),
                headline: row.get("headline"),
                input_fingerprint: row.get("input_fingerprint"),
                generated_at_ms: row.get("generated_at_ms"),
            })
            .collect())
    }

    /// Insert or replace the **Digest** for one `(range_kind, range_start_ms)`
    /// range: a fresh generation overwrites the previous narrative, headline,
    /// fingerprint, `range_end_ms`, and `generated_at_ms` in place (the UNIQUE
    /// index from migration 0029 is the upsert key). `headline` is `None` when
    /// generation produced no usable headline — narrative-only stays valid.
    pub async fn upsert_digest(
        &self,
        range_kind: &str,
        range_start_ms: i64,
        range_end_ms: i64,
        narrative: &str,
        headline: Option<&str>,
        input_fingerprint: &str,
        generated_at_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO user_context_digests \
                (range_kind, range_start_ms, range_end_ms, narrative, headline, \
                 input_fingerprint, generated_at_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT (range_kind, range_start_ms) DO UPDATE SET \
                range_end_ms = excluded.range_end_ms, \
                narrative = excluded.narrative, \
                headline = excluded.headline, \
                input_fingerprint = excluded.input_fingerprint, \
                generated_at_ms = excluded.generated_at_ms",
        )
        .bind(range_kind)
        .bind(range_start_ms)
        .bind(range_end_ms)
        .bind(narrative)
        .bind(headline)
        .bind(input_fingerprint)
        .bind(generated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
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
    /// 2. DELETE every **Digest** whose `[range_start_ms, range_end_ms)` window
    ///    overlaps a to-be-deleted Activity's `[started_at_ms, ended_at_ms]`
    ///    span — a stale narrative could otherwise still describe the deleted
    ///    content. The deleted-capture window is not passed in here; the deleted
    ///    Activities ARE the proxy for it: a digest narrates only Activities, so
    ///    a digest can describe deleted captures only through an Activity
    ///    grounded in them, and every such Activity is in this delete set.
    /// 3. DELETE those Activities — their `*_activity_evidence` and
    ///    `*_conclusion_evidence` link rows cascade via FK.
    /// 4. Re-apply the formation bar: DROP every Conclusion whose surviving
    ///    support-stance evidence falls below
    ///    [`confidence::FORMATION_BAR_EVIDENCE`](super::confidence::FORMATION_BAR_EVIDENCE)
    ///    — evidence loss un-forms a Conclusion the bar would never have let
    ///    form. A pinned Conclusion is exempt down to one surviving support;
    ///    zero surviving support always drops (no ungrounded Conclusions).
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

        // 2. Purge Digests overlapping a to-be-deleted Activity's span, BEFORE
        //    the Activities are deleted (their spans are the overlap source).
        //    Overlap of digest [range_start_ms, range_end_ms) with activity
        //    [started_at_ms, ended_at_ms]: started < range_end AND ended >= start.
        let mut deleted_digests = 0_i64;
        for chunk in activity_ids.chunks(SQLITE_BIND_CHUNK_SIZE) {
            let mut query = QueryBuilder::<Sqlite>::new(
                "DELETE FROM user_context_digests \
                 WHERE EXISTS (\
                     SELECT 1 FROM user_context_activities a \
                     WHERE a.started_at_ms < user_context_digests.range_end_ms \
                       AND a.ended_at_ms >= user_context_digests.range_start_ms \
                       AND a.id IN (",
            );
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(id);
            }
            separated.push_unseparated("))");
            deleted_digests += query.build().execute(&mut *tx).await?.rows_affected() as i64;
        }

        // 3. DELETE those Activities; activity_evidence + conclusion_evidence rows
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

        // 4. Re-apply the formation bar (#95) to the survivors: drop every
        //    Conclusion whose remaining SUPPORT-stance evidence no longer meets
        //    [`confidence::FORMATION_BAR_EVIDENCE`] — losing evidence un-forms a
        //    Conclusion the same way lacking it would have prevented forming one.
        //    A *pinned* Conclusion ("this is true, keep it") is exempt down to a
        //    floor of one support; ZERO surviving support always drops, pinned or
        //    not (ADR 0029: no ungrounded Conclusions, ever).
        let deleted_conclusions = sqlx::query(
            "DELETE FROM user_context_conclusions \
             WHERE (\
                 SELECT COUNT(*) FROM user_context_conclusion_evidence ce \
                 WHERE ce.conclusion_id = user_context_conclusions.id \
                   AND ce.stance = 'support'\
             ) < CASE WHEN pinned = 1 THEN 1 ELSE ?1 END",
        )
        .bind(super::confidence::FORMATION_BAR_EVIDENCE as i64)
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;

        tx.commit().await?;
        Ok(UserContextCascadeSummary {
            deleted_activities,
            deleted_conclusions,
            deleted_digests,
        })
    }

    /// **Wipe User Context** storage half (ADR 0029): in ONE transaction, clear
    /// every `user_context_*` table — all derived **Activity** / **Conclusion**
    /// data, **Dismissal State**, the derivation-run ledger, AND **user-authored
    /// Context** (#107). Raw captures and settings are untouched (this only owns
    /// the dossier tables); the engine is turned off by the Tauri command, not
    /// here. Deletes children before parents to stay correct regardless of FK
    /// enforcement.
    pub async fn wipe_all(&self) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for table in [
            // Children first (leaf evidence / history), then parents, then the
            // FK-free dismissal + derivation-run + authored ledgers.
            "user_context_activity_evidence",
            "user_context_conclusion_evidence",
            "user_context_confidence_history",
            "user_context_activities",
            "user_context_conclusions",
            "user_context_dismissals",
            "user_context_derivation_runs",
            "user_context_authored",
            "user_context_digests",
        ] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

/// Deterministic fingerprint of a **Digest**'s input: the in-range Activity
/// set the narrative was (or would be) derived from. Plain
/// `"v{N}:{count}:{max_id}:{max_created_at_ms}:{accumulator}"` formatting — the
/// accumulator is an order-independent wrapping SUM of one mixed term per
/// Activity (id, span, effective Category/Focus), so element order never
/// matters but membership and per-Activity content do. The Tauri layer compares
/// this against the stored `input_fingerprint` to decide regeneration. The
/// version tag tracks the generated SHAPE (see the comment in the body).
///
/// What invalidates a digest (changes the fingerprint):
/// - an Activity ADDED to the range (new derivation / backfill): `count`,
///   `max_id`, and the accumulator all move;
/// - an Activity REMOVED from the range (Delete Recent Capture cascade —
///   though overlapping digests are also deleted outright there): `count` and
///   the accumulator move;
/// - a Category/Focus CORRECTION (#108) changing an Activity's *effective*
///   label: that Activity's accumulator term moves.
///
/// Honest limitation: `user_context_activities` rows expose no updated-at on
/// the [`Activity`] DTO (`corrected_at_ms` stays in the row), so a correction
/// that lands back on the previous effective value (correct → revert) is
/// invisible — which is fine, because the derivation input is then literally
/// identical. Title/summary are immutable after insert and are not folded in.
pub fn digest_input_fingerprint(activities: &[Activity]) -> String {
    let count = activities.len();
    let max_id = activities.iter().map(|a| a.id).max().unwrap_or(0);
    let max_created_at_ms = activities.iter().map(|a| a.created_at_ms).max().unwrap_or(0);
    let accumulator = activities
        .iter()
        .fold(0_u64, |acc, a| acc.wrapping_add(digest_activity_term(a)));
    // The leading version tag fingerprints the digest's GENERATED SHAPE, not its
    // input: bumping it mismatches every stored `input_fingerprint` at once, so
    // every cached digest regenerates on next view. `v2` added the headline
    // (migration 0030) — pre-headline narratives regenerate WITH one instead of
    // sitting cached forever. Bump it again whenever the generated shape changes.
    format!("v2:{count}:{max_id}:{max_created_at_ms}:{accumulator:016x}")
}

/// One Activity's order-independent accumulator term for
/// [`digest_input_fingerprint`]: its id/span mixed by odd multipliers (so
/// swapping fields between Activities changes the sum) plus the bytes of its
/// effective Category/Focus labels folded in at distinct rotations.
fn digest_activity_term(activity: &Activity) -> u64 {
    let mut term = (activity.id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    term ^= (activity.started_at_ms as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    term ^= (activity.ended_at_ms as u64).wrapping_mul(0x94D0_49BB_1331_11EB);
    let category = activity.category.map(category_to_str).unwrap_or("");
    let focus = activity.focus.map(focus_to_str).unwrap_or("");
    for (rotation, label) in [(7_u32, category), (13_u32, focus)] {
        for byte in label.bytes() {
            term = term.rotate_left(rotation) ^ u64::from(byte);
        }
    }
    term
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
        ActivityCategory::Creating => "creating",
        ActivityCategory::Communication => "communication",
        ActivityCategory::Meetings => "meetings",
        ActivityCategory::Research => "research",
        ActivityCategory::Learning => "learning",
        ActivityCategory::Organizing => "organizing",
        ActivityCategory::Personal => "personal",
        ActivityCategory::Entertainment => "entertainment",
    }
}

/// Parses a stored category string back to an [`ActivityCategory`]; unknown /
/// NULL values map to `None`.
fn category_from_str(value: Option<&str>) -> Option<ActivityCategory> {
    match value {
        Some("creating") => Some(ActivityCategory::Creating),
        Some("communication") => Some(ActivityCategory::Communication),
        Some("meetings") => Some(ActivityCategory::Meetings),
        Some("research") => Some(ActivityCategory::Research),
        Some("learning") => Some(ActivityCategory::Learning),
        Some("organizing") => Some(ActivityCategory::Organizing),
        Some("personal") => Some(ActivityCategory::Personal),
        Some("entertainment") => Some(ActivityCategory::Entertainment),
        _ => None,
    }
}

/// The stored snake_case string for a [`FocusLevel`] (matches the capture-types
/// serde rename and the SQL `focus` column).
fn focus_to_str(focus: FocusLevel) -> &'static str {
    match focus {
        FocusLevel::Deep => "deep",
        FocusLevel::Mixed => "mixed",
        FocusLevel::Distracted => "distracted",
    }
}

/// Parses a stored focus string back to a [`FocusLevel`]; unknown / NULL values
/// map to `None`.
fn focus_from_str(value: Option<&str>) -> Option<FocusLevel> {
    match value {
        Some("deep") => Some(FocusLevel::Deep),
        Some("mixed") => Some(FocusLevel::Mixed),
        Some("distracted") => Some(FocusLevel::Distracted),
        _ => None,
    }
}

/// Map a `user_context_activities` row onto an [`Activity`], applying the #108
/// correction precedence: a user correction WINS over the engine label. When
/// the `*_corrected` flag is set the corrected value (which may be NULL =
/// deliberately "unset") is the effective value; otherwise the engine column is.
fn map_activity(row: SqliteRow) -> Activity {
    let category = if row.get::<i64, _>("category_corrected") != 0 {
        category_from_str(row.get::<Option<String>, _>("corrected_category").as_deref())
    } else {
        category_from_str(row.get::<Option<String>, _>("category").as_deref())
    };
    let focus = if row.get::<i64, _>("focus_corrected") != 0 {
        focus_from_str(row.get::<Option<String>, _>("corrected_focus").as_deref())
    } else {
        focus_from_str(row.get::<Option<String>, _>("focus").as_deref())
    };
    Activity {
        id: row.get("id"),
        title: row.get("title"),
        summary: row.get("summary"),
        category,
        focus,
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
                ungrounded INTEGER NOT NULL DEFAULT 0,
                guardrail_suppressed INTEGER NOT NULL DEFAULT 0,
                below_formation_bar INTEGER NOT NULL DEFAULT 0,
                resurface_blocked INTEGER NOT NULL DEFAULT 0,
                created_at_ms INTEGER NOT NULL
            )",
            "CREATE TABLE user_context_activities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                category TEXT,
                focus TEXT,
                corrected_category TEXT,
                category_corrected INTEGER NOT NULL DEFAULT 0,
                corrected_focus TEXT,
                focus_corrected INTEGER NOT NULL DEFAULT 0,
                corrected_at_ms INTEGER,
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
            "CREATE TABLE user_context_authored (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                topic TEXT,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            )",
            "CREATE TABLE user_context_digests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                range_kind TEXT NOT NULL,
                range_start_ms INTEGER NOT NULL,
                range_end_ms INTEGER NOT NULL,
                narrative TEXT NOT NULL,
                headline TEXT,
                input_fingerprint TEXT NOT NULL,
                generated_at_ms INTEGER NOT NULL
            )",
            "CREATE UNIQUE INDEX user_context_digests_range_idx
                ON user_context_digests (range_kind, range_start_ms)",
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
                focus: None,
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
                focus: None,
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
    fn delete_derived_for_capture_subjects_reapplies_formation_bar() {
        block_on(async {
        let store = test_store().await;

        // Activities A and C are grounded in capture subjects that will be
        // deleted (frame 10, audio 30); B and D are grounded in frame 20,
        // which survives.
        let activity_a = seed_activity_with_subject(&store, "Worked on the spec", 100, "frame", 10).await;
        let activity_b = seed_activity_with_subject(&store, "Reviewed a PR", 200, "frame", 20).await;
        let activity_c = seed_activity_with_subject(&store, "Call about the spec", 300, "audio_segment", 30).await;
        let activity_d = seed_activity_with_subject(&store, "Merged the PR", 400, "frame", 20).await;

        // Conclusion 1: grounded ONLY by A -> zero surviving support -> dropped.
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

        // Conclusion 2: grounded by A and B; loses A and keeps ONE support —
        // below the formation bar (≥2), unpinned -> dropped.
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

        // Conclusion 3: grounded ONLY by C (audio) -> dropped.
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

        // Conclusion 4: grounded by B and D (both survive) -> still meets the
        // formation bar -> stays.
        let b_and_d = store
            .upsert_conclusion(draft("Reviews", "Reviews code carefully", 0.6))
            .await
            .expect("b_and_d");
        store
            .replace_conclusion_evidence(
                b_and_d,
                vec![
                    NewConclusionEvidence { activity_id: activity_b, stance: EvidenceStance::Support },
                    NewConclusionEvidence { activity_id: activity_d, stance: EvidenceStance::Support },
                ],
            )
            .await
            .expect("evidence b_and_d");

        // Conclusion 5: PINNED, grounded by A and B; keeps one support. The pin
        // ("this is true, keep it") exempts it from the formation-bar re-check
        // as long as ≥1 support survives -> stays.
        let pinned = store
            .upsert_conclusion(draft("Focus", "Works in long focus blocks", 0.8))
            .await
            .expect("pinned");
        store
            .replace_conclusion_evidence(
                pinned,
                vec![
                    NewConclusionEvidence { activity_id: activity_a, stance: EvidenceStance::Support },
                    NewConclusionEvidence { activity_id: activity_b, stance: EvidenceStance::Support },
                ],
            )
            .await
            .expect("evidence pinned");
        store.set_pinned(pinned, true).await.expect("pin");

        // Conclusion 6: PINNED but grounded ONLY by A — a pin never overrides
        // the evidence floor; zero surviving support -> dropped.
        let pinned_ungrounded = store
            .upsert_conclusion(draft("Specs", "Lives in spec documents", 0.9))
            .await
            .expect("pinned_ungrounded");
        store
            .replace_conclusion_evidence(
                pinned_ungrounded,
                vec![NewConclusionEvidence { activity_id: activity_a, stance: EvidenceStance::Support }],
            )
            .await
            .expect("evidence pinned_ungrounded");
        store.set_pinned(pinned_ungrounded, true).await.expect("pin ungrounded");

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

        // Activities A and C dropped; B and D survive.
        assert_eq!(summary.deleted_activities, 2, "A (frame 10) and C (audio 30)");
        assert!(store.list_recent_activities(100, 0).await.expect("activities")
            .iter().all(|a| a.id != activity_a && a.id != activity_c));
        assert!(store.list_recent_activities(100, 0).await.expect("activities")
            .iter().any(|a| a.id == activity_b), "B (frame 20) survives");

        // only_a / only_c (no support left) and a_and_b (one support, below the
        // bar, unpinned) dropped; b_and_d (two supports) and pinned (one
        // support but pinned) stay.
        assert_eq!(summary.deleted_conclusions, 4,
            "only_a, only_c, a_and_b, pinned_ungrounded dropped");
        assert!(store.get_conclusion(only_a).await.expect("get").is_none());
        assert!(store.get_conclusion(only_c).await.expect("get").is_none());
        assert!(store.get_conclusion(a_and_b).await.expect("get").is_none(),
            "one surviving support is below the formation bar");
        assert!(store.get_conclusion(pinned_ungrounded).await.expect("get").is_none(),
            "a pin never keeps a Conclusion with zero surviving support");
        let kept = store.get_conclusion(b_and_d).await.expect("get").expect("b_and_d stays");
        assert_eq!(kept.evidence.len(), 2, "both surviving evidence links remain");
        let kept_pinned = store.get_conclusion(pinned).await.expect("get").expect("pinned stays");
        assert_eq!(kept_pinned.evidence.len(), 1, "only B's evidence link remains");
        assert_eq!(kept_pinned.evidence[0].activity_id, activity_b);

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
                    gate_drops: DistillationGateDrops::default(),
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
                gate_drops: DistillationGateDrops::default(),
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

    /// Insert a minimal derivation run for the #113 retry-eligibility tests.
    async fn seed_run(
        store: &UserContextStore,
        kind: &str,
        status: &str,
        window: Option<(i64, i64)>,
    ) {
        store
            .insert_derivation_run(NewDerivationRun {
                kind: kind.to_string(),
                window_start_ms: window.map(|w| w.0),
                window_end_ms: window.map(|w| w.1),
                status: status.to_string(),
                activities_derived: 0,
                conclusions_derived: 0,
                input_tokens: 0,
                output_tokens: 0,
                provider: None,
                model: None,
                error: None,
                gate_drops: DistillationGateDrops::default(),
            })
            .await
            .expect("seed run");
    }

    /// #113 retry-eligibility SQL: a `failed` windowed run is a retryable hole
    /// until a `completed`/`skipped` run covers the same exact span; NULL-bound
    /// failed runs (conclusion/confidence kinds) never qualify.
    #[test]
    fn failed_window_is_eligible_until_a_success_or_skip_covers_it() {
        block_on(async {
            let store = test_store().await;
            // `insert_derivation_run` stamps created_at_ms = now; querying with a
            // far-future backoff anchor makes every failure old enough.
            let no_backoff = i64::MAX;

            seed_run(&store, "activity", "failed", Some((1_000, 2_000))).await;
            // A NULL-bound failed conclusion run must never appear as a window.
            seed_run(&store, "conclusion", "failed", None).await;

            let eligible = store
                .failed_windows_eligible_for_retry(3, no_backoff, 10)
                .await
                .expect("eligible");
            assert_eq!(eligible.len(), 1);
            assert_eq!(eligible[0].kind, "activity");
            assert_eq!(
                (eligible[0].window_start_ms, eligible[0].window_end_ms),
                (1_000, 2_000)
            );
            assert_eq!(eligible[0].failure_count, 1);

            // A completed retry over the same span extinguishes the hole.
            seed_run(&store, "activity", "completed", Some((1_000, 2_000))).await;
            assert!(store
                .failed_windows_eligible_for_retry(3, no_backoff, 10)
                .await
                .expect("after success")
                .is_empty());

            // A `skipped` run covers a span just as well (captures deleted).
            seed_run(&store, "backfill", "failed", Some((3_000, 4_000))).await;
            seed_run(&store, "backfill", "skipped", Some((3_000, 4_000))).await;
            assert!(store
                .failed_windows_eligible_for_retry(3, no_backoff, 10)
                .await
                .expect("after skip")
                .is_empty());
        });
    }

    /// #113 crash-loop backstop: a window with `max_failures` failed runs stops
    /// being eligible (it stays failed and consumes no more engine calls).
    #[test]
    fn failed_window_retry_respects_the_attempt_cap() {
        block_on(async {
            let store = test_store().await;
            let no_backoff = i64::MAX;

            for _ in 0..3 {
                seed_run(&store, "activity", "failed", Some((1_000, 2_000))).await;
            }
            assert!(
                store
                    .failed_windows_eligible_for_retry(3, no_backoff, 10)
                    .await
                    .expect("at cap")
                    .is_empty(),
                "3 failures with a cap of 3 => permanently failed"
            );
            // A higher cap still sees it, with the full failure count.
            let eligible = store
                .failed_windows_eligible_for_retry(4, no_backoff, 10)
                .await
                .expect("below higher cap");
            assert_eq!(eligible.len(), 1);
            assert_eq!(eligible[0].failure_count, 3);
        });
    }

    /// #113 wall-clock backoff: a window whose newest failure is younger than
    /// the backoff anchor is skipped this pass.
    #[test]
    fn failed_window_retry_respects_the_backoff_anchor() {
        block_on(async {
            let store = test_store().await;
            seed_run(&store, "activity", "failed", Some((1_000, 2_000))).await;

            // Anchor in the past => the just-inserted failure is too fresh.
            assert!(store
                .failed_windows_eligible_for_retry(3, now_ms() - 60_000, 10)
                .await
                .expect("fresh failure")
                .is_empty());
            // Anchor at/after the failure time => eligible.
            assert_eq!(
                store
                    .failed_windows_eligible_for_retry(3, i64::MAX, 10)
                    .await
                    .expect("aged failure")
                    .len(),
                1
            );
        });
    }

    /// #113 ordering: eligible holes come back newest-first (matching the
    /// History Backfill policy) and the limit caps the pass.
    #[test]
    fn failed_window_retry_is_newest_first_and_limited() {
        block_on(async {
            let store = test_store().await;
            let no_backoff = i64::MAX;

            seed_run(&store, "backfill", "failed", Some((1_000, 2_000))).await;
            seed_run(&store, "activity", "failed", Some((5_000, 6_000))).await;

            let eligible = store
                .failed_windows_eligible_for_retry(3, no_backoff, 1)
                .await
                .expect("limited");
            assert_eq!(eligible.len(), 1);
            assert_eq!(
                (eligible[0].window_start_ms, eligible[0].window_end_ms),
                (5_000, 6_000),
                "the newest hole retries first"
            );
            assert_eq!(eligible[0].kind, "activity");
        });
    }

    /// The settings readout's "why is my dossier thin?" line reads the newest
    /// COMPLETED `'conclusion'` run's per-gate withheld counts; failed and
    /// non-conclusion runs never shadow it.
    #[test]
    fn latest_distillation_summary_reads_newest_completed_conclusion_run() {
        block_on(async {
            let store = test_store().await;
            assert!(
                store.latest_distillation_summary().await.expect("empty").is_none(),
                "no distillation yet => None"
            );

            let conclusion_run = |status: &str, derived: i64, drops: DistillationGateDrops| {
                NewDerivationRun {
                    kind: "conclusion".to_string(),
                    window_start_ms: None,
                    window_end_ms: None,
                    status: status.to_string(),
                    activities_derived: 0,
                    conclusions_derived: derived,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: None,
                    model: None,
                    error: None,
                    gate_drops: drops,
                }
            };

            // Older completed run with different counts.
            store
                .insert_derivation_run(conclusion_run(
                    "completed",
                    5,
                    DistillationGateDrops { ungrounded: 9, ..Default::default() },
                ))
                .await
                .expect("older run");
            // The newest completed run: this one must win.
            let expected = DistillationGateDrops {
                ungrounded: 1,
                guardrail_suppressed: 2,
                below_formation_bar: 3,
                resurface_blocked: 4,
            };
            store
                .insert_derivation_run(conclusion_run("completed", 2, expected))
                .await
                .expect("newest completed run");
            // A newer FAILED conclusion run and a newer activity run are ignored.
            store
                .insert_derivation_run(conclusion_run("failed", 0, Default::default()))
                .await
                .expect("failed run");
            store
                .insert_derivation_run(NewDerivationRun {
                    kind: "activity".to_string(),
                    window_start_ms: Some(0),
                    window_end_ms: Some(1),
                    status: "completed".to_string(),
                    activities_derived: 1,
                    conclusions_derived: 0,
                    input_tokens: 0,
                    output_tokens: 0,
                    provider: None,
                    model: None,
                    error: None,
                    gate_drops: DistillationGateDrops::default(),
                })
                .await
                .expect("activity run");

            let (_, derived, drops) = store
                .latest_distillation_summary()
                .await
                .expect("summary")
                .expect("a completed conclusion run exists");
            assert_eq!(derived, 2, "newest completed conclusion run's upsert count");
            assert_eq!(drops, expected, "its per-gate withheld counts round-trip");
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
                gate_drops: DistillationGateDrops::default(),
            })
            .await
            .expect("derivation run");
        let to_dismiss = store
            .upsert_conclusion(draft("Vim", "Uses Vim", 0.4))
            .await
            .expect("to dismiss");
        store.dismiss_conclusion(to_dismiss).await.expect("dismiss");
        store
            .add_authored_context("I'm a designer", Some("role"), 1_000)
            .await
            .expect("authored");
        store
            .upsert_digest("week", 0, 1_000, "A focused week.", None, "1:1:1:0", 2_000)
            .await
            .expect("digest");

        // Sanity: everything present before the wipe.
        assert!(store.count_activities().await.expect("count") > 0);
        assert!(store.count_conclusions().await.expect("count") > 0);
        assert!(!store.list_dismissals().await.expect("dismissals").is_empty());
        assert!(!store.list_authored_context().await.expect("authored").is_empty());
        assert!(store.get_digest("week", 0).await.expect("digest").is_some());

        store.wipe_all().await.expect("wipe");

        // Every table is empty.
        assert_eq!(store.count_activities().await.expect("count"), 0);
        assert_eq!(store.count_conclusions().await.expect("count"), 0);
        assert!(store.list_dismissals().await.expect("dismissals").is_empty());
        assert!(store.list_authored_context().await.expect("authored").is_empty());
        for table in [
            "user_context_activity_evidence",
            "user_context_conclusion_evidence",
            "user_context_confidence_history",
            "user_context_activities",
            "user_context_conclusions",
            "user_context_dismissals",
            "user_context_derivation_runs",
            "user_context_authored",
            "user_context_digests",
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

    #[test]
    fn authored_context_add_list_update_delete_round_trip() {
        block_on(async {
            let store = test_store().await;

            // Add two statements; list is newest-first by created_at_ms.
            let first = store
                .add_authored_context("I'm a designer", Some("role"), 1_000)
                .await
                .expect("add first");
            let second = store
                .add_authored_context("I care about typography", None, 2_000)
                .await
                .expect("add second");

            let listed = store.list_authored_context().await.expect("list");
            assert_eq!(listed.len(), 2);
            assert_eq!(listed[0].id, second, "newest first");
            assert_eq!(listed[0].text, "I care about typography");
            assert_eq!(listed[0].topic, None);
            assert_eq!(listed[1].id, first);
            assert_eq!(listed[1].topic.as_deref(), Some("role"));
            assert_eq!(listed[1].created_at_ms, 1_000);
            assert_eq!(listed[1].updated_at_ms, 1_000, "updated == created on insert");

            // Update bumps text/topic and updated_at_ms but keeps created_at_ms.
            store
                .update_authored_context(first, "I'm a product designer", Some("job"), 5_000)
                .await
                .expect("update");
            let updated = store
                .list_authored_context()
                .await
                .expect("list after update")
                .into_iter()
                .find(|c| c.id == first)
                .expect("present");
            assert_eq!(updated.text, "I'm a product designer");
            assert_eq!(updated.topic.as_deref(), Some("job"));
            assert_eq!(updated.created_at_ms, 1_000, "created_at_ms unchanged");
            assert_eq!(updated.updated_at_ms, 5_000, "updated_at_ms bumped");

            // Delete removes only the named row.
            store.delete_authored_context(first).await.expect("delete");
            let remaining = store.list_authored_context().await.expect("list after delete");
            assert_eq!(remaining.len(), 1);
            assert_eq!(remaining[0].id, second);

            // Deleting an absent id is a no-op.
            store.delete_authored_context(9999).await.expect("noop delete");
            assert_eq!(store.list_authored_context().await.expect("list").len(), 1);
        });
    }

    #[test]
    fn delete_recent_cascade_leaves_authored_context_intact() {
        block_on(async {
            let store = test_store().await;

            // An Activity + Conclusion grounded in frame 10 (to be deleted), and a
            // user-authored statement that must survive the cascade.
            let activity = seed_activity_with_subject(&store, "Designed a thing", 100, "frame", 10).await;
            let conclusion = store
                .upsert_conclusion(draft("Design", "Cares about design", 0.7))
                .await
                .expect("conclusion");
            store
                .replace_conclusion_evidence(
                    conclusion,
                    vec![NewConclusionEvidence { activity_id: activity, stance: EvidenceStance::Support }],
                )
                .await
                .expect("evidence");
            store
                .add_authored_context("I'm a designer", Some("role"), 1_000)
                .await
                .expect("authored");

            // Delete frame 10: the derived Activity + ungrounded Conclusion drop.
            let summary = store
                .delete_derived_for_capture_subjects(&[10], &[])
                .await
                .expect("cascade");
            assert_eq!(summary.deleted_activities, 1);
            assert_eq!(summary.deleted_conclusions, 1);

            // The user-authored statement is NOT derived from any capture, so the
            // cascade leaves it untouched.
            let authored = store.list_authored_context().await.expect("authored after cascade");
            assert_eq!(authored.len(), 1, "authored Context survives Delete Recent cascade");
            assert_eq!(authored[0].text, "I'm a designer");
        });
    }

    /// #105: engine-assigned Category + Focus persist on insert and read back
    /// (effective values equal the engine labels when uncorrected).
    #[test]
    fn category_and_focus_persist_and_read_back() {
        block_on(async {
            let store = test_store().await;
            let id = store
                .insert_activity_with_evidence(NewActivity {
                    title: "Wrote the parser".to_string(),
                    summary: "Implemented the tokenizer".to_string(),
                    category: Some(ActivityCategory::Creating),
                    focus: Some(FocusLevel::Deep),
                    started_at_ms: 100,
                    ended_at_ms: 200,
                    derivation_run_id: None,
                    evidence: vec![NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 10,
                        captured_at_ms: Some(100),
                    }],
                })
                .await
                .expect("insert");

            let activities = store.list_recent_activities(10, 0).await.expect("list");
            let activity = activities.iter().find(|a| a.id == id).expect("present");
            assert_eq!(activity.category, Some(ActivityCategory::Creating));
            assert_eq!(activity.focus, Some(FocusLevel::Deep));

            // Same effective values via the distillation read path.
            let distill = store.activities_for_distillation(10).await.expect("distill");
            let from_distill = distill.iter().find(|a| a.id == id).expect("present");
            assert_eq!(from_distill.category, Some(ActivityCategory::Creating));
            assert_eq!(from_distill.focus, Some(FocusLevel::Deep));
        });
    }

    /// #108: a user correction WINS over the engine label and survives a
    /// re-persist (a fresh engine label on a NEW row never silently overwrites a
    /// correction — corrections are per-row, and the override columns are not
    /// touched by `insert_activity_with_evidence`). Also covers correcting to
    /// `None` ("unset"), which the `*_corrected` flag distinguishes from
    /// "never corrected".
    #[test]
    fn correction_overrides_engine_label_and_persists() {
        block_on(async {
            let store = test_store().await;
            let id = store
                .insert_activity_with_evidence(NewActivity {
                    title: "Scrolled social media".to_string(),
                    summary: "Browsed feeds".to_string(),
                    category: Some(ActivityCategory::Research),
                    focus: Some(FocusLevel::Mixed),
                    started_at_ms: 100,
                    ended_at_ms: 200,
                    derivation_run_id: None,
                    evidence: vec![NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 10,
                        captured_at_ms: Some(100),
                    }],
                })
                .await
                .expect("insert");

            // Correct Category Research -> Entertainment and Focus Mixed -> Distracted.
            store
                .correct_activity(
                    id,
                    Some(Some(ActivityCategory::Entertainment)),
                    Some(Some(FocusLevel::Distracted)),
                )
                .await
                .expect("correct");

            let activity = store
                .list_recent_activities(10, 0)
                .await
                .expect("list")
                .into_iter()
                .find(|a| a.id == id)
                .expect("present");
            assert_eq!(
                activity.category,
                Some(ActivityCategory::Entertainment),
                "corrected category wins over engine Research"
            );
            assert_eq!(
                activity.focus,
                Some(FocusLevel::Distracted),
                "corrected focus wins over engine Mixed"
            );

            // It shows up in the corrections feed (newest first), carrying the
            // effective corrected values + the title/summary for the prompt.
            let corrections = store.list_activity_corrections(10).await.expect("corrections");
            assert_eq!(corrections.len(), 1);
            assert_eq!(corrections[0].activity_id, id);
            assert_eq!(corrections[0].title, "Scrolled social media");
            assert_eq!(corrections[0].corrected_category, Some(ActivityCategory::Entertainment));
            assert_eq!(corrections[0].corrected_focus, Some(FocusLevel::Distracted));

            // Simulate the engine re-deriving the SAME activity into a fresh row
            // with its (wrong) label again. Corrections are per-row state on the
            // existing corrected row, so the new engine row does not touch them:
            // the corrected row's effective values are unchanged.
            let _fresh = store
                .insert_activity_with_evidence(NewActivity {
                    title: "Scrolled social media".to_string(),
                    summary: "Browsed feeds".to_string(),
                    category: Some(ActivityCategory::Research),
                    focus: Some(FocusLevel::Mixed),
                    started_at_ms: 300,
                    ended_at_ms: 400,
                    derivation_run_id: None,
                    evidence: vec![NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 11,
                        captured_at_ms: Some(300),
                    }],
                })
                .await
                .expect("fresh insert");
            let still_corrected = store
                .list_recent_activities(10, 0)
                .await
                .expect("list")
                .into_iter()
                .find(|a| a.id == id)
                .expect("present");
            assert_eq!(still_corrected.category, Some(ActivityCategory::Entertainment));
            assert_eq!(still_corrected.focus, Some(FocusLevel::Distracted));

            // Correcting Category to None ("unset") wins over the engine label too:
            // the flag is set, so the effective category is None, NOT Research.
            store
                .correct_activity(id, Some(None), None)
                .await
                .expect("correct to none");
            let unset = store
                .list_recent_activities(10, 0)
                .await
                .expect("list")
                .into_iter()
                .find(|a| a.id == id)
                .expect("present");
            assert_eq!(unset.category, None, "corrected-to-None wins over engine Research");
            // Focus correction (Distracted) is untouched by the category-only correction.
            assert_eq!(unset.focus, Some(FocusLevel::Distracted));
        });
    }

    /// #108: `correct_activity` with both args `None` is a no-op (no correction
    /// recorded, no timestamp), so an uncorrected Activity stays on its engine
    /// label and never appears in the corrections feed.
    #[test]
    fn correct_activity_noop_when_nothing_supplied() {
        block_on(async {
            let store = test_store().await;
            let id = store
                .insert_activity_with_evidence(NewActivity {
                    title: "Reviewed a PR".to_string(),
                    summary: "Looked at the diff".to_string(),
                    category: Some(ActivityCategory::Creating),
                    focus: Some(FocusLevel::Deep),
                    started_at_ms: 100,
                    ended_at_ms: 200,
                    derivation_run_id: None,
                    evidence: vec![NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 10,
                        captured_at_ms: Some(100),
                    }],
                })
                .await
                .expect("insert");

            store.correct_activity(id, None, None).await.expect("noop");

            let activity = store
                .list_recent_activities(10, 0)
                .await
                .expect("list")
                .into_iter()
                .find(|a| a.id == id)
                .expect("present");
            assert_eq!(activity.category, Some(ActivityCategory::Creating));
            assert_eq!(activity.focus, Some(FocusLevel::Deep));
            assert!(
                store.list_activity_corrections(10).await.expect("corrections").is_empty(),
                "no correction recorded"
            );
        });
    }

    /// ADR 0032: the store layer round-trips exactly the eight
    /// profession-neutral work modes; old v1 labels (relabeled once by
    /// migration 0031) and unknown strings map to `None`.
    #[test]
    fn category_strings_round_trip_the_fixed_taxonomy() {
        for category in [
            ActivityCategory::Creating,
            ActivityCategory::Communication,
            ActivityCategory::Meetings,
            ActivityCategory::Research,
            ActivityCategory::Learning,
            ActivityCategory::Organizing,
            ActivityCategory::Personal,
            ActivityCategory::Entertainment,
        ] {
            assert_eq!(category_from_str(Some(category_to_str(category))), Some(category));
        }
        for old in ["coding", "testing", "design", "distractions"] {
            assert_eq!(
                category_from_str(Some(old)),
                None,
                "old v1 label {old:?} is migration-only, not a store alias"
            );
        }
        assert_eq!(category_from_str(None), None);
    }

    /// Structural guarantee for migration 0031 (ADR 0032): every old v1 label
    /// that changes meaning is relabeled in BOTH the engine `category` column
    /// and the #108 `corrected_category` column.
    #[test]
    fn generalize_categories_migration_relabels_both_columns() {
        let migration =
            include_str!("../../migrations/0031_generalize_activity_categories.sql");
        let statements: Vec<&str> = migration.split(';').collect();
        for column in ["category", "corrected_category"] {
            for (old, new) in [
                ("coding", "creating"),
                ("testing", "creating"),
                ("design", "creating"),
                ("distractions", "entertainment"),
            ] {
                assert!(
                    statements.iter().any(|statement| {
                        statement.contains(&format!("SET {column} = '{new}'"))
                            && statement.contains(&format!("WHERE {column}"))
                            && statement.contains(&format!("'{old}'"))
                    }),
                    "migration 0031 must relabel {column} {old:?} -> {new:?}"
                );
            }
        }
    }

    /// A bare [`Activity`] value for the pure [`digest_input_fingerprint`]
    /// tests (the fingerprint never reads title/summary/evidence).
    fn digest_activity(id: i64, started_at_ms: i64, ended_at_ms: i64) -> Activity {
        Activity {
            id,
            title: format!("activity {id}"),
            summary: String::new(),
            category: None,
            focus: None,
            started_at_ms,
            ended_at_ms,
            created_at_ms: started_at_ms,
            evidence: Vec::new(),
        }
    }

    /// [`UserContextStore::list_activities_in_range`] selects exactly the
    /// Activities overlapping the half-open range, oldest first, with no
    /// evidence hydration.
    #[test]
    fn list_activities_in_range_uses_half_open_overlap_oldest_first() {
        block_on(async {
            let store = test_store().await;
            // seed_activity spans [started, started + 1].
            seed_activity(&store, "before", 500).await; // ends 501 < 1_000 → out
            seed_activity(&store, "touches-start", 999).await; // ends 1_000 → in
            seed_activity(&store, "inside", 1_500).await; // in
            seed_activity(&store, "at-end", 2_000).await; // starts AT end → out (half-open)

            let in_range = store
                .list_activities_in_range(1_000, 2_000)
                .await
                .expect("range query");
            let titles: Vec<&str> = in_range.iter().map(|a| a.title.as_str()).collect();
            assert_eq!(titles, vec!["touches-start", "inside"], "overlap + order");
            assert!(
                in_range.iter().all(|a| a.evidence.is_empty()),
                "digest input does not hydrate evidence"
            );
        });
    }

    /// Digest round trip: get miss → upsert → get hit → upsert overwrites the
    /// narrative/headline/fingerprint/range_end/generated_at in place (same
    /// key, including headline Some → None), and a different `range_kind` with
    /// the same start is a separate row.
    #[test]
    fn digest_round_trip_upsert_overwrites_in_place() {
        block_on(async {
            let store = test_store().await;

            assert!(store.get_digest("week", 100).await.expect("miss").is_none());

            store
                .upsert_digest(
                    "week",
                    100,
                    200,
                    "A focused week.",
                    Some("A deep week in the editor"),
                    "2:7:90:00ab",
                    1_000,
                )
                .await
                .expect("upsert");
            let stored = store.get_digest("week", 100).await.expect("get").expect("hit");
            assert_eq!(
                stored,
                StoredDigest {
                    range_kind: "week".to_string(),
                    range_start_ms: 100,
                    range_end_ms: 200,
                    narrative: "A focused week.".to_string(),
                    headline: Some("A deep week in the editor".to_string()),
                    input_fingerprint: "2:7:90:00ab".to_string(),
                    generated_at_ms: 1_000,
                }
            );

            // Same (range_kind, range_start_ms) key → in-place replacement; a
            // headline-less regeneration clears the previous headline.
            store
                .upsert_digest("week", 100, 250, "A scattered week.", None, "3:9:240:00cd", 2_000)
                .await
                .expect("overwrite");
            let replaced = store.get_digest("week", 100).await.expect("get").expect("hit");
            assert_eq!(replaced.range_end_ms, 250);
            assert_eq!(replaced.narrative, "A scattered week.");
            assert_eq!(replaced.headline, None);
            assert_eq!(replaced.input_fingerprint, "3:9:240:00cd");
            assert_eq!(replaced.generated_at_ms, 2_000);

            // A different range_kind at the same start is its own row.
            store
                .upsert_digest("day", 100, 150, "A quiet day.", None, "1:1:100:0001", 3_000)
                .await
                .expect("day digest");
            assert_eq!(
                store.get_digest("week", 100).await.expect("get").expect("hit").narrative,
                "A scattered week."
            );
            assert_eq!(
                store.get_digest("day", 100).await.expect("get").expect("hit").narrative,
                "A quiet day."
            );
        });
    }

    /// `list_day_digests_in_range` returns only the DAY digests whose half-open
    /// span overlaps the query window, chronologically — the day that falls
    /// entirely outside the window is excluded.
    #[test]
    fn list_day_digests_in_range_returns_only_overlapping() {
        block_on(async {
            let store = test_store().await;

            // Inside the [1_000, 2_000) window.
            store
                .upsert_digest("day", 1_000, 1_500, "An overlapping day.", None, "fp-in", 10)
                .await
                .expect("inside digest");
            // Entirely after the window (starts at its end).
            store
                .upsert_digest("day", 2_000, 2_500, "A later day.", None, "fp-out", 20)
                .await
                .expect("outside digest");

            let digests = store
                .list_day_digests_in_range(1_000, 2_000)
                .await
                .expect("list");

            assert_eq!(digests.len(), 1);
            assert_eq!(
                digests[0],
                StoredDigest {
                    range_kind: "day".to_string(),
                    range_start_ms: 1_000,
                    range_end_ms: 1_500,
                    narrative: "An overlapping day.".to_string(),
                    headline: None,
                    input_fingerprint: "fp-in".to_string(),
                    generated_at_ms: 10,
                }
            );
        });
    }

    /// [`digest_input_fingerprint`] is deterministic and order-independent over
    /// the same Activity set, and moves when the set or any Activity's content
    /// (membership, timestamps, effective Category/Focus correction) changes.
    #[test]
    fn digest_input_fingerprint_is_order_independent_and_change_sensitive() {
        let a = digest_activity(1, 100, 200);
        let b = digest_activity(2, 300, 400);
        let c = digest_activity(3, 500, 600);

        // Deterministic + order-independent.
        let baseline = digest_input_fingerprint(&[a.clone(), b.clone()]);
        assert_eq!(baseline, digest_input_fingerprint(&[a.clone(), b.clone()]));
        assert_eq!(baseline, digest_input_fingerprint(&[b.clone(), a.clone()]));

        // Membership changes move it: added, removed, empty.
        assert_ne!(baseline, digest_input_fingerprint(&[a.clone(), b.clone(), c]));
        assert_ne!(baseline, digest_input_fingerprint(&[a.clone()]));
        // The `v2:` shape-version tag leads every fingerprint (see the body
        // comment): bumping it invalidates every cached digest at once.
        assert_eq!(digest_input_fingerprint(&[]), "v2:0:0:0:0000000000000000");
        assert!(baseline.starts_with("v2:"), "version tag leads: {baseline}");

        // A timestamp shift on one Activity moves it.
        let shifted = digest_activity(2, 300, 450);
        assert_ne!(baseline, digest_input_fingerprint(&[a.clone(), shifted]));

        // A #108 correction changing the EFFECTIVE Category/Focus moves it.
        let mut corrected = b.clone();
        corrected.category = Some(ActivityCategory::Entertainment);
        assert_ne!(baseline, digest_input_fingerprint(&[a.clone(), corrected]));
        let mut refocused = b.clone();
        refocused.focus = Some(FocusLevel::Deep);
        assert_ne!(baseline, digest_input_fingerprint(&[a, refocused]));
    }

    /// The Delete Recent Capture cascade purges every Digest whose
    /// `[range_start_ms, range_end_ms)` window overlaps a deleted Activity's
    /// span, and spares non-overlapping Digests (including those overlapping
    /// only SURVIVING Activities).
    #[test]
    fn delete_derived_cascade_purges_overlapping_digests_only() {
        block_on(async {
            let store = test_store().await;

            // Deleted: grounded in frame 10, span [1_000, 2_000].
            store
                .insert_activity_with_evidence(NewActivity {
                    title: "Sensitive thing".to_string(),
                    summary: "Sensitive thing summary".to_string(),
                    category: None,
                    focus: None,
                    started_at_ms: 1_000,
                    ended_at_ms: 2_000,
                    derivation_run_id: None,
                    evidence: vec![NewActivityEvidence {
                        subject_type: "frame".to_string(),
                        subject_id: 10,
                        captured_at_ms: Some(1_000),
                    }],
                })
                .await
                .expect("sensitive activity");
            // Survives: grounded in frame 20, span [10_000, 10_001].
            seed_activity_with_subject(&store, "Kept work", 10_000, "frame", 20).await;

            // d1 overlaps the deleted span outright.
            store
                .upsert_digest("day", 0, 5_000, "Mentions the sensitive thing.", None, "fp1", 1)
                .await
                .expect("d1");
            // d2 touches the deleted span only at its boundary (activity ends at
            // 2_000 == range_start): still an overlap (ended_at_ms inclusive).
            store
                .upsert_digest("week", 2_000, 8_000, "Also mentions it.", None, "fp2", 1)
                .await
                .expect("d2");
            // d3 sits strictly between the two Activities: no overlap, spared.
            store
                .upsert_digest("day", 5_000, 9_000, "Quiet stretch.", None, "fp3", 1)
                .await
                .expect("d3");
            // d4 overlaps only the SURVIVING Activity: spared.
            store
                .upsert_digest("day", 9_500, 12_000, "About the kept work.", None, "fp4", 1)
                .await
                .expect("d4");

            let summary = store
                .delete_derived_for_capture_subjects(&[10], &[])
                .await
                .expect("cascade");
            assert_eq!(summary.deleted_activities, 1);
            assert_eq!(summary.deleted_digests, 2, "d1 and d2 purged");

            assert!(store.get_digest("day", 0).await.expect("d1").is_none());
            assert!(store.get_digest("week", 2_000).await.expect("d2").is_none());
            assert!(store.get_digest("day", 5_000).await.expect("d3").is_some());
            assert!(store.get_digest("day", 9_500).await.expect("d4").is_some());

            // An empty-ids cascade never touches digests.
            let noop = store
                .delete_derived_for_capture_subjects(&[], &[])
                .await
                .expect("noop");
            assert_eq!(noop.deleted_digests, 0);
            assert!(store.get_digest("day", 5_000).await.expect("d3").is_some());
        });
    }
}
