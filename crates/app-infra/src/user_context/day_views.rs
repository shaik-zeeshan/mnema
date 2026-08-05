//! Read-time day projections for the Overview redesign (slice 9):
//! conversations-for-day and moments-for-day.
//!
//! **No migration, no new tables.** A "conversation" is not an entity — it is
//! an existing `user_context_activities` row whose time-overlapping
//! `speaker_turns` clear [`CONVERSATION_MIN_SPEECH_MS`] (DECISIONS.md
//! "Conversations & moments"). Because everything is computed at read time the
//! projection is retroactive over existing data by construction, and the
//! Activity row is never mutated (the spill-extended end is display-only).
//!
//! `speaker_turns.start_ms`/`end_ms` are **segment-relative** offsets; absolute
//! turn time = the owning `audio_segments.started_at` (legacy RFC3339 TEXT,
//! converted to unix-millis at the boundary, matching `capture_source`) plus
//! the offset.

use sqlx::Row;

use capture_types::{DayConversation, DayMoment};

use crate::{Result, FRAME_SUBJECT_TYPE};

use super::capture_source::{ms_to_rfc3339, rfc3339_to_ms};
use super::store::UserContextStore;

/// The one knob (DECISIONS.md, grill 2026-08-04): an Activity is a conversation
/// when its total overlapping turn time is **at or above** 2 minutes. No
/// minimum speaker count.
pub const CONVERSATION_MIN_SPEECH_MS: i64 = 2 * 60 * 1_000;

/// One diarized turn with absolute unix-millis bounds, ready for overlap math.
struct AbsoluteTurn {
    start_ms: i64,
    end_ms: i64,
    cluster_id: i64,
}

impl UserContextStore {
    /// Activities overlapping the half-open `[range_start_ms, range_end_ms)`
    /// window that qualify as conversations, chronological (oldest first).
    ///
    /// Overlap semantics: a turn overlaps an Activity when their ranges
    /// intersect with positive length (`turn_start < act_end && turn_end >
    /// act_start` — a turn merely touching a boundary contributes nothing);
    /// its contribution is the length of the intersection. Concurrent turns
    /// (two speakers at once) each contribute fully — `speech_ms` is summed
    /// turn time, not wall-clock coverage.
    ///
    /// Spill: a turn that STARTS inside the Activity window but ends after it
    /// extends `display_ended_at_ms` to the latest such turn end. The Activity
    /// row itself is never touched.
    pub async fn conversations_for_day(
        &self,
        range_start_ms: i64,
        range_end_ms: i64,
    ) -> Result<Vec<DayConversation>> {
        let activities = self
            .list_activities_in_range(range_start_ms, range_end_ms)
            .await?;
        if activities.is_empty() {
            return Ok(Vec::new());
        }

        // One turn fetch for the whole activity span. Any turn overlapping (or
        // spilling out of) an in-range Activity lives in a segment overlapping
        // this span, so nothing relevant is missed.
        let span_start_ms = activities.iter().map(|a| a.started_at_ms).min().unwrap_or(0);
        let span_end_ms = activities.iter().map(|a| a.ended_at_ms).max().unwrap_or(0);
        let turns = self.fetch_absolute_turns(span_start_ms, span_end_ms).await?;

        // ponytail: O(activities × turns) scan — a day is dozens of activities
        // × a few thousand turns; sort+sweep only if a profiler ever complains.
        let mut out = Vec::new();
        for activity in activities {
            let mut speech_ms = 0i64;
            let mut display_end_ms = activity.ended_at_ms;
            let mut clusters = std::collections::HashSet::new();
            for turn in &turns {
                let overlap = turn.end_ms.min(activity.ended_at_ms)
                    - turn.start_ms.max(activity.started_at_ms);
                if overlap <= 0 {
                    continue;
                }
                speech_ms += overlap;
                clusters.insert(turn.cluster_id);
                // Spill: starts inside the window, ends after it.
                if turn.start_ms >= activity.started_at_ms && turn.end_ms > activity.ended_at_ms {
                    display_end_ms = display_end_ms.max(turn.end_ms);
                }
            }
            if speech_ms >= CONVERSATION_MIN_SPEECH_MS {
                out.push(DayConversation {
                    activity_id: activity.id,
                    title: activity.title,
                    started_at_ms: activity.started_at_ms,
                    ended_at_ms: activity.ended_at_ms,
                    display_ended_at_ms: display_end_ms,
                    speaker_count: clusters.len() as i64,
                    speech_ms,
                });
            }
        }
        Ok(out)
    }

    /// The day's Activities' headline frames (`is_headline` evidence, migration
    /// `0046`) for the moments strip, best first by a dumb focus-weight ×
    /// duration rule (deep 3 / mixed 2 / distracted-or-unknown 1; user
    /// correction wins). Activities whose headline frame was aged out by
    /// Retention (or that have none) simply drop out — no frame, no moment.
    pub async fn moments_for_day(
        &self,
        range_start_ms: i64,
        range_end_ms: i64,
    ) -> Result<Vec<DayMoment>> {
        let rows = sqlx::query(
            "SELECT a.id AS activity_id, a.title, \
                    a.started_at_ms, a.ended_at_ms, \
                    CASE WHEN a.focus_corrected != 0 THEN a.corrected_focus ELSE a.focus END \
                        AS effective_focus, \
                    e.captured_at_ms AS evidence_captured_at_ms, \
                    f.id AS frame_id, f.file_path, f.captured_at AS frame_captured_at \
             FROM user_context_activities a \
             JOIN user_context_activity_evidence e \
                ON e.activity_id = a.id AND e.is_headline = 1 AND e.subject_type = ?3 \
             JOIN frames f ON f.id = e.subject_id \
             WHERE a.started_at_ms < ?2 AND a.ended_at_ms >= ?1",
        )
        .bind(range_start_ms)
        .bind(range_end_ms)
        .bind(FRAME_SUBJECT_TYPE)
        .fetch_all(self.read_pool())
        .await?;

        let mut moments: Vec<(i64, DayMoment)> = rows
            .into_iter()
            .map(|row| {
                let started_at_ms: i64 = row.get("started_at_ms");
                let ended_at_ms: i64 = row.get("ended_at_ms");
                let weight = match row.get::<Option<String>, _>("effective_focus").as_deref() {
                    Some("deep") => 3,
                    Some("mixed") => 2,
                    _ => 1,
                };
                let captured_at_ms = row
                    .get::<Option<i64>, _>("evidence_captured_at_ms")
                    .or_else(|| rfc3339_to_ms(&row.get::<String, _>("frame_captured_at")))
                    .unwrap_or(started_at_ms);
                (
                    weight * (ended_at_ms - started_at_ms).max(0),
                    DayMoment {
                        activity_id: row.get("activity_id"),
                        activity_title: row.get("title"),
                        frame_id: row.get("frame_id"),
                        frame_path: row.get("file_path"),
                        captured_at_ms,
                    },
                )
            })
            .collect();
        // Best score first; ties chronological for a stable strip.
        moments.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.captured_at_ms.cmp(&b.1.captured_at_ms)));
        Ok(moments.into_iter().map(|(_, m)| m).collect())
    }

    /// All turns whose owning segment overlaps `[span_start_ms, span_end_ms]`,
    /// as absolute unix-millis. Driven from `audio_segments` (index seek on
    /// `audio_segments_time_range_idx`, then per-segment turns) — mirroring the
    /// `brokered_access::speakers` CROSS JOIN note. Bounds are padded 1s so the
    /// RFC3339 TEXT comparison quirks at exact-second boundaries cannot drop a
    /// segment; the precise overlap math happens in Rust anyway.
    async fn fetch_absolute_turns(
        &self,
        span_start_ms: i64,
        span_end_ms: i64,
    ) -> Result<Vec<AbsoluteTurn>> {
        let rows = sqlx::query(
            "SELECT segment.started_at AS segment_started_at, \
                    turn.start_ms, turn.end_ms, turn.cluster_id \
             FROM audio_segments segment \
             CROSS JOIN speaker_turns turn ON turn.audio_segment_id = segment.id \
             WHERE segment.started_at <= ?2 AND segment.ended_at >= ?1",
        )
        .bind(ms_to_rfc3339(span_start_ms - 1_000))
        .bind(ms_to_rfc3339(span_end_ms + 1_000))
        .fetch_all(self.read_pool())
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let segment_start =
                    rfc3339_to_ms(&row.get::<String, _>("segment_started_at"))?;
                Some(AbsoluteTurn {
                    start_ms: segment_start + row.get::<i64, _>("start_ms"),
                    end_ms: segment_start + row.get::<i64, _>("end_ms"),
                    cluster_id: row.get("cluster_id"),
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::CaptureDb;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    /// Minimal fixture schema: just the columns the read queries touch.
    async fn fixture_store() -> UserContextStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory db");
        for ddl in [
            "CREATE TABLE user_context_activities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                category TEXT,
                focus TEXT,
                corrected_category TEXT,
                category_corrected INTEGER NOT NULL DEFAULT 0,
                corrected_focus TEXT,
                focus_corrected INTEGER NOT NULL DEFAULT 0,
                started_at_ms INTEGER NOT NULL,
                ended_at_ms INTEGER NOT NULL,
                created_at_ms INTEGER NOT NULL DEFAULT 0
            )",
            "CREATE TABLE user_context_activity_evidence (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                activity_id INTEGER NOT NULL,
                subject_type TEXT NOT NULL,
                subject_id INTEGER NOT NULL,
                captured_at_ms INTEGER,
                is_headline INTEGER NOT NULL DEFAULT 0
            )",
            "CREATE TABLE audio_segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL
            )",
            "CREATE TABLE speaker_turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                audio_segment_id INTEGER NOT NULL,
                cluster_id INTEGER NOT NULL,
                start_ms INTEGER NOT NULL,
                end_ms INTEGER NOT NULL
            )",
            "CREATE TABLE frames (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL,
                captured_at TEXT NOT NULL
            )",
        ] {
            sqlx::query(ddl).execute(&pool).await.expect("create table");
        }
        UserContextStore::new(CaptureDb::single(pool))
    }

    /// Day window: 2026-01-01 UTC. Segment starts at 10:00:00.
    const DAY_START_MS: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z
    const DAY_END_MS: i64 = DAY_START_MS + 86_400_000;
    const SEG_START_MS: i64 = DAY_START_MS + 10 * 3_600_000;

    async fn insert_activity(store: &UserContextStore, title: &str, start: i64, end: i64) -> i64 {
        sqlx::query(
            "INSERT INTO user_context_activities (title, started_at_ms, ended_at_ms) \
             VALUES (?1, ?2, ?3)",
        )
        .bind(title)
        .bind(start)
        .bind(end)
        .execute(store.read_pool())
        .await
        .expect("insert activity")
        .last_insert_rowid()
    }

    /// One segment covering [SEG_START_MS, SEG_START_MS + 1h].
    async fn insert_segment(pool: &SqlitePool) -> i64 {
        sqlx::query("INSERT INTO audio_segments (started_at, ended_at) VALUES (?1, ?2)")
            .bind(ms_to_rfc3339(SEG_START_MS))
            .bind(ms_to_rfc3339(SEG_START_MS + 3_600_000))
            .execute(pool)
            .await
            .expect("insert segment")
            .last_insert_rowid()
    }

    /// Turn bounds are given in ABSOLUTE ms and stored segment-relative.
    async fn insert_turn(pool: &SqlitePool, segment_id: i64, cluster: i64, abs_start: i64, abs_end: i64) {
        sqlx::query(
            "INSERT INTO speaker_turns (audio_segment_id, cluster_id, start_ms, end_ms) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(segment_id)
        .bind(cluster)
        .bind(abs_start - SEG_START_MS)
        .bind(abs_end - SEG_START_MS)
        .execute(pool)
        .await
        .expect("insert turn");
    }

    #[test]
    fn two_minute_bar_below_at_above() {
        block_on(async {
            let store = fixture_store().await;
            let act_start = SEG_START_MS;
            let act_end = SEG_START_MS + 30 * 60_000;
            // Three activities sharing the window shape, staggered so each
            // only overlaps its own turns.
            let below = insert_activity(&store, "below", act_start, act_end).await;
            let at_start = act_end + 60_000;
            let at_end = at_start + 30 * 60_000;
            let at = insert_activity(&store, "at", at_start, at_end).await;
            let seg = insert_segment(store.read_pool()).await;
            // below: 119_999 ms of speech.
            insert_turn(store.read_pool(), seg, 1, act_start, act_start + 119_999).await;
            // at: exactly 120_000 ms — included (>=).
            insert_turn(store.read_pool(), seg, 1, at_start, at_start + 120_000).await;

            let conversations = store
                .conversations_for_day(DAY_START_MS, DAY_END_MS)
                .await
                .expect("query");
            assert_eq!(
                conversations.iter().map(|c| c.activity_id).collect::<Vec<_>>(),
                vec![at],
                "119_999 ms is below the bar, 120_000 ms is at it"
            );
            assert_eq!(conversations[0].speech_ms, 120_000);
            let _ = below;

            // Above: only the in-window intersection counts. A turn from 1 min
            // BEFORE the activity to 3 min in contributes 3 min, not 4.
            let above_start = at_end + 60 * 60_000;
            let above_end = above_start + 30 * 60_000;
            let above = insert_activity(&store, "above", above_start, above_end).await;
            let seg2 = insert_segment(store.read_pool()).await; // same span, fine
            insert_turn(store.read_pool(), seg2, 1, above_start - 60_000, above_start + 180_000)
                .await;
            let conversations = store
                .conversations_for_day(DAY_START_MS, DAY_END_MS + 86_400_000)
                .await
                .expect("query");
            let above_row = conversations
                .iter()
                .find(|c| c.activity_id == above)
                .expect("above the bar via clipped intersection");
            assert_eq!(above_row.speech_ms, 180_000);
        });
    }

    #[test]
    fn speaker_count_is_distinct_clusters() {
        block_on(async {
            let store = fixture_store().await;
            let act_start = SEG_START_MS;
            let act = insert_activity(&store, "meeting", act_start, act_start + 30 * 60_000).await;
            let seg = insert_segment(store.read_pool()).await;
            // Cluster 1 twice + cluster 2 once => 2 speakers.
            insert_turn(store.read_pool(), seg, 1, act_start, act_start + 90_000).await;
            insert_turn(store.read_pool(), seg, 1, act_start + 100_000, act_start + 190_000).await;
            insert_turn(store.read_pool(), seg, 2, act_start + 200_000, act_start + 260_000).await;

            let conversations = store
                .conversations_for_day(DAY_START_MS, DAY_END_MS)
                .await
                .expect("query");
            assert_eq!(conversations.len(), 1);
            assert_eq!(conversations[0].activity_id, act);
            assert_eq!(conversations[0].speaker_count, 2, "same cluster counts once");
        });
    }

    #[test]
    fn spill_extends_displayed_duration_only() {
        block_on(async {
            let store = fixture_store().await;
            let act_start = SEG_START_MS;
            let act_end = act_start + 10 * 60_000;
            let act = insert_activity(&store, "call", act_start, act_end).await;
            let seg = insert_segment(store.read_pool()).await;
            // Clears the bar inside the window, then a turn starting inside
            // ends 90 s past the activity end.
            insert_turn(store.read_pool(), seg, 1, act_start, act_start + 150_000).await;
            let spill_end = act_end + 90_000;
            insert_turn(store.read_pool(), seg, 2, act_end - 30_000, spill_end).await;

            let conversations = store
                .conversations_for_day(DAY_START_MS, DAY_END_MS)
                .await
                .expect("query");
            assert_eq!(conversations.len(), 1);
            assert_eq!(conversations[0].display_ended_at_ms, spill_end);
            assert_eq!(conversations[0].ended_at_ms, act_end, "wire shape keeps the row's end");

            // The Activity row itself is untouched.
            let stored_end: i64 =
                sqlx::query_scalar("SELECT ended_at_ms FROM user_context_activities WHERE id = ?1")
                    .bind(act)
                    .fetch_one(store.read_pool())
                    .await
                    .expect("row");
            assert_eq!(stored_end, act_end);
        });
    }

    #[test]
    fn zero_turn_activity_excluded() {
        block_on(async {
            let store = fixture_store().await;
            insert_activity(&store, "silent coding", SEG_START_MS, SEG_START_MS + 3_600_000).await;
            let conversations = store
                .conversations_for_day(DAY_START_MS, DAY_END_MS)
                .await
                .expect("query");
            assert!(conversations.is_empty());
        });
    }

    #[test]
    fn moments_order_by_focus_weight_times_duration() {
        block_on(async {
            let store = fixture_store().await;
            let pool = store.read_pool().clone();
            // deep 10 min (score 30) vs mixed 20 min (score 40) vs
            // no-focus 60 min (score 60); one activity without a headline frame
            // drops out.
            let deep = insert_activity(&store, "deep", DAY_START_MS, DAY_START_MS + 600_000).await;
            let mixed =
                insert_activity(&store, "mixed", DAY_START_MS, DAY_START_MS + 1_200_000).await;
            let unfocused =
                insert_activity(&store, "none", DAY_START_MS, DAY_START_MS + 3_600_000).await;
            let frameless =
                insert_activity(&store, "frameless", DAY_START_MS, DAY_START_MS + 3_600_000).await;
            sqlx::query("UPDATE user_context_activities SET focus = 'deep' WHERE id = ?1")
                .bind(deep)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE user_context_activities SET focus = 'mixed' WHERE id = ?1")
                .bind(mixed)
                .execute(&pool)
                .await
                .unwrap();
            for (i, activity_id) in [deep, mixed, unfocused].iter().enumerate() {
                let frame_id: i64 = sqlx::query(
                    "INSERT INTO frames (file_path, captured_at) VALUES (?1, ?2)",
                )
                .bind(format!("/frames/{i}.jpg"))
                .bind(ms_to_rfc3339(DAY_START_MS + i as i64 * 1_000))
                .execute(&pool)
                .await
                .unwrap()
                .last_insert_rowid();
                sqlx::query(
                    "INSERT INTO user_context_activity_evidence \
                        (activity_id, subject_type, subject_id, captured_at_ms, is_headline) \
                     VALUES (?1, 'frame', ?2, ?3, 1)",
                )
                .bind(activity_id)
                .bind(frame_id)
                .bind(DAY_START_MS + i as i64 * 1_000)
                .execute(&pool)
                .await
                .unwrap();
            }

            let moments = store
                .moments_for_day(DAY_START_MS, DAY_END_MS)
                .await
                .expect("query");
            assert_eq!(
                moments.iter().map(|m| m.activity_id).collect::<Vec<_>>(),
                vec![unfocused, mixed, deep],
                "ordered by focus weight × duration, frameless activity dropped"
            );
            let _ = frameless;
            assert_eq!(moments[0].frame_path, "/frames/2.jpg");
            assert_eq!(moments[0].activity_title, "none");
        });
    }
}
