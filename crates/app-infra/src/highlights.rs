//! Read-time day highlights: **conversations** and **moments**.
//!
//! No new tables and no pipeline stage — both surfaces are queries over data the
//! capture/derivation pipeline already wrote:
//!
//! - **Conversations** are a read-time join of `user_context_activities` against
//!   `speaker_turns`. An Activity counts as a conversation when speech overlaps
//!   its window for at least [`CONVERSATION_MIN_OVERLAP_MS`]. Speaker turn
//!   offsets (`speaker_turns.start_ms` / `end_ms`) are **segment-relative**, so
//!   the absolute clock comes from `audio_segments.started_at` (RFC3339 TEXT).
//!
//! - **Moments** are the frames the Activity derivation flagged as headline
//!   evidence (`user_context_activity_evidence.is_headline = 1` with
//!   `subject_type = 'frame'`), ranked by the Activity's effective focus band
//!   and then by how long the Activity lasted.
//!
//! Both take a half-open `[start_ms, end_ms)` window and use overlap semantics
//! (`started_at_ms < end AND ended_at_ms > start`), so an Activity straddling a
//! day boundary appears in both days rather than falling through the crack.

use sqlx::{Row, SqlitePool};
use std::collections::HashSet;

use capture_types::{ConversationCluster, Moment};

use crate::db::CaptureDb;
use crate::user_context::capture_source::{ms_to_rfc3339, rfc3339_to_ms};
use crate::user_context::store::focus_from_str;
use crate::{Result, FRAME_SUBJECT_TYPE};

/// Minimum summed speech overlap for an Activity to be reported as a
/// conversation (2 minutes). Below this it is background chatter or a one-line
/// interruption, not a conversation worth a tile.
pub const CONVERSATION_MIN_OVERLAP_MS: i64 = 120_000;

/// Default number of moments returned when the caller does not cap it.
pub const DEFAULT_MOMENTS_LIMIT: i64 = 12;

/// Read store for the day-highlight surfaces. Mirrors `UsageChartsStore`: a
/// thin wrapper over the shared reader pool, no state of its own.
#[derive(Clone)]
pub struct HighlightsStore {
    db: CaptureDb,
}

/// One Activity window pulled for the conversation join.
#[derive(Debug, Clone)]
struct ActivityWindow {
    id: i64,
    title: String,
    started_at_ms: i64,
    ended_at_ms: i64,
}

/// One speaker turn resolved onto the absolute clock.
#[derive(Debug, Clone, Copy)]
struct AbsoluteTurn {
    cluster_id: i64,
    start_ms: i64,
    end_ms: i64,
}

impl HighlightsStore {
    pub fn new(db: CaptureDb) -> Self {
        Self { db }
    }

    fn pool(&self) -> &SqlitePool {
        self.db.read()
    }

    /// Conversations overlapping the half-open window `[start_ms, end_ms)`,
    /// newest first. Only Activities with at least
    /// [`CONVERSATION_MIN_OVERLAP_MS`] of speech overlap are returned.
    pub async fn conversations(
        &self,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<ConversationCluster>> {
        let activities = self.fetch_activity_windows(start_ms, end_ms).await?;
        if activities.is_empty() {
            return Ok(Vec::new());
        }
        // Turns are fetched over the ACTIVITY extent, not the query window: an
        // Activity straddling the window edge keeps its whole window here, so
        // clipping its speech to the query window would under-report it (and
        // make the same conversation look shorter on one day than the other).
        let turns_start = activities
            .iter()
            .map(|a| a.started_at_ms)
            .min()
            .unwrap_or(start_ms);
        let turns_end = activities
            .iter()
            .map(|a| a.ended_at_ms)
            .max()
            .unwrap_or(end_ms);
        let turns = self.fetch_absolute_turns(turns_start, turns_end).await?;
        Ok(cluster_conversations(&activities, &turns))
    }

    /// Headline-frame moments for Activities overlapping `[start_ms, end_ms)`,
    /// ranked by focus band then Activity duration. `limit` defaults to
    /// [`DEFAULT_MOMENTS_LIMIT`] and is clamped to at least 1.
    pub async fn moments(
        &self,
        start_ms: i64,
        end_ms: i64,
        limit: Option<i64>,
    ) -> Result<Vec<Moment>> {
        let limit = limit.unwrap_or(DEFAULT_MOMENTS_LIMIT).max(1);

        // `focus_rank` is emitted as a column so ORDER BY can sort on the
        // corrected-focus expression without repeating the CASE ladder.
        let rows = sqlx::query(
            "SELECT frames.id AS frame_id, \
                    frames.file_path AS file_path, \
                    frames.captured_at AS captured_at, \
                    activities.id AS activity_id, \
                    activities.title AS title, \
                    activities.started_at_ms AS started_at_ms, \
                    activities.ended_at_ms AS ended_at_ms, \
                    CASE WHEN activities.focus_corrected != 0 \
                         THEN activities.corrected_focus ELSE activities.focus END AS focus, \
                    CASE CASE WHEN activities.focus_corrected != 0 \
                              THEN activities.corrected_focus ELSE activities.focus END \
                         WHEN 'deep' THEN 0 WHEN 'mixed' THEN 1 WHEN 'distracted' THEN 2 \
                         ELSE 3 END AS focus_rank \
             FROM user_context_activity_evidence AS evidence \
             JOIN user_context_activities AS activities ON activities.id = evidence.activity_id \
             JOIN frames ON frames.id = evidence.subject_id \
             WHERE evidence.subject_type = ?1 AND evidence.is_headline != 0 \
               AND activities.started_at_ms < ?3 AND activities.ended_at_ms > ?2 \
             ORDER BY focus_rank ASC, \
                      (activities.ended_at_ms - activities.started_at_ms) DESC, \
                      activities.started_at_ms DESC, frames.id ASC \
             LIMIT ?4",
        )
        .bind(FRAME_SUBJECT_TYPE)
        .bind(start_ms)
        .bind(end_ms)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let captured_at: String = row.get("captured_at");
            let Some(captured_at_ms) = rfc3339_to_ms(&captured_at) else {
                continue;
            };
            let started_at_ms: i64 = row.get("started_at_ms");
            let ended_at_ms: i64 = row.get("ended_at_ms");
            out.push(Moment {
                frame_id: row.get("frame_id"),
                file_path: row.get("file_path"),
                captured_at_ms,
                activity_id: row.get("activity_id"),
                title: row.get("title"),
                focus: focus_from_str(row.get::<Option<String>, _>("focus").as_deref()),
                activity_started_at_ms: started_at_ms,
                activity_ended_at_ms: ended_at_ms,
                duration_ms: (ended_at_ms - started_at_ms).max(0),
            });
        }
        Ok(out)
    }

    /// Activities overlapping `[start_ms, end_ms)`, newest first.
    async fn fetch_activity_windows(
        &self,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<ActivityWindow>> {
        let rows = sqlx::query(
            "SELECT id, title, started_at_ms, ended_at_ms FROM user_context_activities \
             WHERE started_at_ms < ?2 AND ended_at_ms > ?1 \
             ORDER BY started_at_ms DESC, id DESC",
        )
        .bind(start_ms)
        .bind(end_ms)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| ActivityWindow {
                id: row.get("id"),
                title: row.get("title"),
                started_at_ms: row.get("started_at_ms"),
                ended_at_ms: row.get("ended_at_ms"),
            })
            .collect())
    }

    /// Speaker turns whose owning audio segment overlaps `[start_ms, end_ms)`,
    /// lifted onto the absolute clock. The segment filter is deliberately
    /// coarse (a turn near a segment edge can fall just outside the window);
    /// the per-Activity overlap arithmetic clamps it back.
    async fn fetch_absolute_turns(&self, start_ms: i64, end_ms: i64) -> Result<Vec<AbsoluteTurn>> {
        let rows = sqlx::query(
            "SELECT audio_segments.started_at AS segment_started_at, \
                    speaker_turns.cluster_id AS cluster_id, \
                    speaker_turns.start_ms AS start_ms, \
                    speaker_turns.end_ms AS end_ms \
             FROM speaker_turns \
             JOIN audio_segments ON audio_segments.id = speaker_turns.audio_segment_id \
             WHERE audio_segments.started_at < ?2 AND audio_segments.ended_at > ?1",
        )
        .bind(ms_to_rfc3339(start_ms))
        .bind(ms_to_rfc3339(end_ms))
        .fetch_all(self.pool())
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let segment_started_at: String = row.get("segment_started_at");
            let Some(segment_start_ms) = rfc3339_to_ms(&segment_started_at) else {
                continue;
            };
            let offset_start: i64 = row.get("start_ms");
            let offset_end: i64 = row.get("end_ms");
            out.push(AbsoluteTurn {
                cluster_id: row.get("cluster_id"),
                start_ms: segment_start_ms + offset_start,
                end_ms: segment_start_ms + offset_end,
            });
        }
        Ok(out)
    }
}

/// Pure join: for each Activity, sum the overlap of every speaker turn against
/// its window and count the distinct speaker clusters that contributed. Keeps
/// only Activities at or above [`CONVERSATION_MIN_OVERLAP_MS`], preserving the
/// input (newest-first) order.
///
/// A cluster only counts as a speaker when it actually overlaps the window —
/// a turn tangent to the boundary (zero overlap) neither adds time nor a
/// speaker.
///
// ponytail: nested scan, O(activities × turns). Both are per-window and small
// (a day is tens of activities and hundreds of turns); sort both by start and
// sweep if a multi-month window ever becomes a real caller.
fn cluster_conversations(
    activities: &[ActivityWindow],
    turns: &[AbsoluteTurn],
) -> Vec<ConversationCluster> {
    let mut out = Vec::new();
    for activity in activities {
        let mut spoken_ms = 0i64;
        let mut speakers: HashSet<i64> = HashSet::new();
        for turn in turns {
            let overlap =
                turn.end_ms.min(activity.ended_at_ms) - turn.start_ms.max(activity.started_at_ms);
            if overlap <= 0 {
                continue;
            }
            spoken_ms += overlap;
            speakers.insert(turn.cluster_id);
        }
        if spoken_ms < CONVERSATION_MIN_OVERLAP_MS {
            continue;
        }
        out.push(ConversationCluster {
            activity_id: activity.id,
            title: activity.title.clone(),
            started_at_ms: activity.started_at_ms,
            ended_at_ms: activity.ended_at_ms,
            spoken_ms,
            speaker_count: speakers.len() as i64,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_types::FocusLevel;
    use sqlx::sqlite::SqlitePoolOptions;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }

    fn activity(id: i64, started_at_ms: i64, ended_at_ms: i64) -> ActivityWindow {
        ActivityWindow {
            id,
            title: format!("Activity {id}"),
            started_at_ms,
            ended_at_ms,
        }
    }

    fn turn(cluster_id: i64, start_ms: i64, end_ms: i64) -> AbsoluteTurn {
        AbsoluteTurn {
            cluster_id,
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn overlap_exactly_two_minutes_is_a_conversation() {
        let activities = vec![activity(1, 0, 600_000)];
        let turns = vec![turn(10, 0, CONVERSATION_MIN_OVERLAP_MS)];
        let clusters = cluster_conversations(&activities, &turns);
        assert_eq!(clusters.len(), 1, "the >= boundary must be inclusive");
        assert_eq!(clusters[0].spoken_ms, CONVERSATION_MIN_OVERLAP_MS);
    }

    #[test]
    fn overlap_one_millisecond_short_is_not_a_conversation() {
        let activities = vec![activity(1, 0, 600_000)];
        let turns = vec![turn(10, 0, CONVERSATION_MIN_OVERLAP_MS - 1)];
        assert!(cluster_conversations(&activities, &turns).is_empty());
    }

    #[test]
    fn overlap_is_clipped_to_the_activity_window() {
        // A 10-minute turn that only pokes 90s into the Activity is below the
        // threshold, even though the turn itself is far longer than 2 minutes.
        let activities = vec![activity(1, 0, 90_000)];
        let turns = vec![turn(10, -510_000, 120_000)];
        assert!(cluster_conversations(&activities, &turns).is_empty());

        // Widen the Activity so the clipped overlap reaches the threshold.
        let activities = vec![activity(1, 0, CONVERSATION_MIN_OVERLAP_MS)];
        let clusters = cluster_conversations(&activities, &turns);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].spoken_ms, CONVERSATION_MIN_OVERLAP_MS);
    }

    #[test]
    fn short_turns_accumulate_across_the_threshold() {
        // Four 30s turns = exactly 2 minutes: no single turn qualifies alone.
        let activities = vec![activity(1, 0, 600_000)];
        let turns = vec![
            turn(10, 0, 30_000),
            turn(11, 60_000, 90_000),
            turn(10, 120_000, 150_000),
            turn(11, 180_000, 210_000),
        ];
        let clusters = cluster_conversations(&activities, &turns);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].spoken_ms, 120_000);
        assert_eq!(clusters[0].speaker_count, 2, "two distinct clusters");
    }

    #[test]
    fn speaker_count_ignores_non_overlapping_and_tangent_turns() {
        let activities = vec![activity(1, 0, 600_000)];
        let turns = vec![
            turn(10, 0, 200_000),
            // Tangent at the start boundary: zero overlap, no speaker.
            turn(11, -50_000, 0),
            // Entirely after the Activity.
            turn(12, 700_000, 800_000),
        ];
        let clusters = cluster_conversations(&activities, &turns);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].speaker_count, 1);
        assert_eq!(clusters[0].spoken_ms, 200_000);
    }

    #[test]
    fn each_activity_is_scored_independently() {
        let activities = vec![activity(2, 600_000, 1_200_000), activity(1, 0, 600_000)];
        let turns = vec![
            turn(10, 0, 130_000),
            turn(11, 600_000, 610_000),
            turn(12, 900_000, 900_000),
        ];
        let clusters = cluster_conversations(&activities, &turns);
        // Activity 2 only has 10s of speech -> dropped. Activity 1 keeps its
        // position in the (newest-first) input order.
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].activity_id, 1);
        assert_eq!(clusters[0].spoken_ms, 130_000);
    }

    /// End-to-end against an in-memory DB with the real table shapes, covering
    /// the segment-relative -> absolute turn conversion and the moments ORDER BY.
    #[test]
    fn highlights_queries_run_against_sqlite() {
        block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db");

            for statement in [
                "CREATE TABLE user_context_activities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    category TEXT,
                    started_at_ms INTEGER NOT NULL,
                    ended_at_ms INTEGER NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    focus TEXT,
                    corrected_focus TEXT,
                    focus_corrected INTEGER NOT NULL DEFAULT 0
                )",
                "CREATE TABLE user_context_activity_evidence (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    activity_id INTEGER NOT NULL,
                    subject_type TEXT NOT NULL,
                    subject_id INTEGER NOT NULL,
                    captured_at_ms INTEGER,
                    is_headline INTEGER NOT NULL DEFAULT 0
                )",
                "CREATE TABLE frames (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    captured_at TEXT NOT NULL
                )",
                "CREATE TABLE audio_segments (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source_kind TEXT NOT NULL,
                    source_session_id TEXT NOT NULL,
                    segment_index INTEGER NOT NULL,
                    file_path TEXT NOT NULL,
                    started_at TEXT NOT NULL,
                    ended_at TEXT NOT NULL
                )",
                "CREATE TABLE speaker_turns (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    audio_segment_id INTEGER NOT NULL,
                    session_id TEXT NOT NULL,
                    cluster_id INTEGER NOT NULL,
                    start_ms INTEGER NOT NULL,
                    end_ms INTEGER NOT NULL
                )",
            ] {
                sqlx::query(statement)
                    .execute(&pool)
                    .await
                    .expect("create table");
            }

            // Two activities on 2026-01-01: a long "deep" meeting (10 min) and a
            // shorter "mixed" one (5 min).
            sqlx::query(
                "INSERT INTO user_context_activities \
                 (id, title, summary, started_at_ms, ended_at_ms, created_at_ms, focus, \
                  corrected_focus, focus_corrected) VALUES \
                 (1, 'Design review', 's', 0, 600000, 0, 'mixed', 'deep', 1), \
                 (2, 'Inbox', 's', 600000, 900000, 0, 'mixed', NULL, 0), \
                 (3, 'Silent reading', 's', 900000, 1800000, 0, 'deep', NULL, 0)",
            )
            .execute(&pool)
            .await
            .expect("insert activities");

            // Segment starts at epoch 0 (RFC3339); turn offsets are relative to
            // it, so turn [30s, 210s) is absolute [30s, 210s) = 180s overlap.
            sqlx::query(
                "INSERT INTO audio_segments \
                 (id, source_kind, source_session_id, segment_index, file_path, started_at, ended_at) \
                 VALUES (1, 'microphone', 'sess', 1, 'a.m4a', '1970-01-01T00:00:00Z', '1970-01-01T00:10:00Z')",
            )
            .execute(&pool)
            .await
            .expect("insert segment");
            sqlx::query(
                "INSERT INTO speaker_turns (audio_segment_id, session_id, cluster_id, start_ms, end_ms) \
                 VALUES (1, 'sess', 10, 30000, 150000), (1, 'sess', 11, 150000, 210000)",
            )
            .execute(&pool)
            .await
            .expect("insert turns");

            sqlx::query(
                "INSERT INTO frames (id, session_id, file_path, captured_at) VALUES \
                 (100, 's1', '/f/deep.jpg', '1970-01-01T00:01:00Z'), \
                 (200, 's1', '/f/mixed.jpg', '1970-01-01T00:11:00Z'), \
                 (300, 's1', '/f/notheadline.jpg', '1970-01-01T00:16:00Z')",
            )
            .execute(&pool)
            .await
            .expect("insert frames");
            sqlx::query(
                "INSERT INTO user_context_activity_evidence \
                 (activity_id, subject_type, subject_id, is_headline) VALUES \
                 (1, 'frame', 100, 1), (2, 'frame', 200, 1), (3, 'frame', 300, 0)",
            )
            .execute(&pool)
            .await
            .expect("insert evidence");

            let store = HighlightsStore::new(CaptureDb::single(pool));

            let conversations = store.conversations(0, 1_800_000).await.expect("conv");
            assert_eq!(conversations.len(), 1, "only activity 1 clears 2 minutes");
            assert_eq!(conversations[0].activity_id, 1);
            assert_eq!(conversations[0].spoken_ms, 180_000);
            assert_eq!(conversations[0].speaker_count, 2);

            // A query window that starts mid-Activity still reports the whole
            // Activity's speech: the window selects Activities, it does not
            // clip them (otherwise a straddling conversation would look
            // shorter on one side of a day boundary than the other).
            let narrow = store.conversations(120_000, 1_800_000).await.expect("conv");
            assert_eq!(narrow.len(), 1);
            assert_eq!(narrow[0].spoken_ms, 180_000);

            // A window that excludes the Activity entirely returns nothing.
            let after = store
                .conversations(1_800_000, 3_600_000)
                .await
                .expect("conv");
            assert!(after.is_empty());

            let moments = store.moments(0, 1_800_000, None).await.expect("moments");
            assert_eq!(moments.len(), 2, "only headline frames");
            // Activity 1's corrected focus ('deep') wins over its engine label
            // ('mixed'), so it sorts ahead of activity 2.
            assert_eq!(moments[0].frame_id, 100);
            assert_eq!(moments[0].focus, Some(FocusLevel::Deep));
            assert_eq!(moments[0].duration_ms, 600_000);
            assert_eq!(moments[0].captured_at_ms, 60_000);
            assert_eq!(moments[1].frame_id, 200);
            assert_eq!(moments[1].focus, Some(FocusLevel::Mixed));

            let capped = store.moments(0, 1_800_000, Some(1)).await.expect("capped");
            assert_eq!(capped.len(), 1);
            assert_eq!(capped[0].frame_id, 100);
        });
    }
}
