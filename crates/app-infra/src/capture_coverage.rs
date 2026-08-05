//! Per-day capture coverage for the timeline's jump menu (round-4 decision G6).
//!
//! ONE `GROUP BY` over the existing `capture_segments` table — no new tables, no
//! new columns. The result is a sparse list of local calendar days that hold
//! capture; a day that is absent has no recording and is disabled everywhere in
//! the jump menu.
//!
//! ## Local day bucketing
//!
//! Days are bucketed by the **frontend-stamped local UTC offset**
//! (`app_settings` key `user_context.local_offset_minutes`, the same value the
//! digest and User Context distillation use — see CLAUDE.md "AI Temporal
//! Grounding"). Never stamped → UTC, exactly like the distillation worker's
//! fallback. The offset is applied as a SQLite datetime modifier, so a day is
//! always 24 hours wide at the stored offset: a DST transition shifts the wall
//! clock but never splits, drops, or duplicates a day here. That is a
//! deliberate simplification — a segment recorded before a DST change is
//! labelled with the offset in force now, which can move it by an hour (and, at
//! a midnight boundary, by a day). The alternative is a full tz database in the
//! query path for a navigation menu.
//!
//! ## `covered_ms` is an estimate
//!
//! The three capture families (screen / microphone / system audio) record
//! concurrently, so summing every segment's duration triple-counts wall time.
//! We instead sum per source within an hour and take the **max across sources**
//! — exact when the sources run together (the common case) and never worse than
//! the widest single source. Each hour is then capped at one hour, since a
//! segment is attributed wholly to the hour it starts in (segments are capped
//! at 5 minutes, so the spill is bounded).
//!
//! ## Caching
//!
//! The query result is cached in process behind a cheap validity stamp
//! (`COUNT(*)`, `MAX(id)`, `MAX(updated_at)` over the same table). Every write
//! that matters — a committed segment, a retention sweep, Delete Recent Capture
//! — moves the stamp, so the cache refreshes without any writer having to
//! remember to invalidate it. There are three separate write sites today (this
//! crate's upsert, its retention cleanup, and a raw `DELETE` in the desktop
//! crate); hooking each is more code and one forgotten hook away from a stale
//! menu.

use std::sync::{Arc, Mutex};

use sqlx::{Row, SqlitePool};

use capture_types::DayCoverage;

use crate::db::CaptureDb;
use crate::user_context::UserContextStore;
use crate::Result;

const HOUR_MS: i64 = 3_600_000;

/// `(row count, max id, max updated_at)` — the cheap fingerprint that tells us
/// whether the coverage query has to run again.
type CoverageStamp = (i64, Option<i64>, Option<String>);

#[derive(Clone)]
pub struct CaptureCoverageStore {
    db: CaptureDb,
    cache: Arc<Mutex<Option<(CoverageStamp, Arc<Vec<DayCoverage>>)>>>,
}

impl CaptureCoverageStore {
    pub fn new(db: CaptureDb) -> Self {
        Self {
            db,
            cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Every local day that holds capture, ascending by day.
    pub async fn day_coverage(&self) -> Result<Arc<Vec<DayCoverage>>> {
        let stamp = coverage_stamp(self.db.read()).await?;
        if let Some(hit) = self.cache.lock().ok().and_then(|c| {
            c.as_ref()
                .filter(|(s, _)| *s == stamp)
                .map(|(_, d)| d.clone())
        }) {
            return Ok(hit);
        }

        let offset_minutes = UserContextStore::new(self.db.clone())
            .local_offset_minutes()
            .await
            .ok()
            .flatten()
            .unwrap_or(0);
        let days = Arc::new(query_day_coverage(self.db.read(), offset_minutes).await?);
        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some((stamp, days.clone()));
        }
        Ok(days)
    }
}

/// SQLite datetime modifier for a UTC offset in minutes (`+330 minutes`).
fn offset_modifier(offset_minutes: i64) -> String {
    format!("{:+} minutes", offset_minutes)
}

async fn coverage_stamp(pool: &SqlitePool) -> Result<CoverageStamp> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n, MAX(id) AS max_id, MAX(updated_at) AS max_updated
         FROM capture_segments",
    )
    .fetch_one(pool)
    .await?;
    Ok((
        row.get::<i64, _>("n"),
        row.get::<Option<i64>, _>("max_id"),
        row.get::<Option<String>, _>("max_updated"),
    ))
}

/// The one GROUP BY. Inner query: per source, per local hour, summed duration.
/// Outer query: the widest source wins that hour (see the module docs on why
/// summing across sources would triple-count).
async fn query_day_coverage(pool: &SqlitePool, offset_minutes: i64) -> Result<Vec<DayCoverage>> {
    let modifier = offset_modifier(offset_minutes);
    let rows = sqlx::query(
        "SELECT day, hour, MAX(source_ms) AS covered_ms
         FROM (
             SELECT strftime('%Y-%m-%d', started_at, ?1) AS day,
                    CAST(strftime('%H', started_at, ?1) AS INTEGER) AS hour,
                    source_kind,
                    -- Whole seconds via strftime('%s'): exact integer arithmetic.
                    -- julianday() differences are doubles and land a millisecond
                    -- short of round numbers; sub-second precision is noise for a
                    -- coverage bar anyway.
                    SUM(MAX(0, (CAST(strftime('%s', ended_at) AS INTEGER)
                                - CAST(strftime('%s', started_at) AS INTEGER)) * 1000))
                        AS source_ms
             FROM capture_segments
             WHERE status <> 'pending_delete'
               AND started_at IS NOT NULL
             GROUP BY day, hour, source_kind
         )
         WHERE day IS NOT NULL
         GROUP BY day, hour
         ORDER BY day ASC, hour ASC",
    )
    .bind(&modifier)
    .fetch_all(pool)
    .await?;

    let mut days: Vec<DayCoverage> = Vec::new();
    for row in rows {
        let day: String = row.get("day");
        let hour: i64 = row.get("hour");
        let covered_ms: i64 = row.get::<Option<i64>, _>("covered_ms").unwrap_or(0);
        let entry = match days.last_mut() {
            Some(last) if last.day == day => last,
            _ => {
                days.push(DayCoverage {
                    day,
                    covered_ms: 0,
                    hours: Vec::new(),
                });
                days.last_mut().expect("just pushed")
            }
        };
        entry.covered_ms += covered_ms.clamp(0, HOUR_MS);
        if (0..=23).contains(&hour) {
            entry.hours.push(hour as u8);
        }
    }
    Ok(days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }

    /// `(source_kind, started_at, ended_at)` rows, inserted with sequential
    /// segment indices so the table's UNIQUE constraint is satisfied.
    async fn pool_with(segments: &[(&str, &str, &str)]) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory db should open");
        sqlx::query(
            "CREATE TABLE capture_segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                capture_session_id TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                source_session_id TEXT NOT NULL,
                segment_index INTEGER NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                status TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(source_kind, source_session_id, segment_index)
            )",
        )
        .execute(&pool)
        .await
        .expect("capture_segments should be created");
        for (index, (kind, start, end)) in segments.iter().enumerate() {
            sqlx::query(
                "INSERT INTO capture_segments
                    (capture_session_id, source_kind, source_session_id, segment_index,
                     started_at, ended_at, status)
                 VALUES ('capture-1', ?1, ?2, ?3, ?4, ?5, 'completed')",
            )
            .bind(kind)
            .bind(format!("{kind}-source"))
            .bind(index as i64 + 1)
            .bind(start)
            .bind(end)
            .execute(&pool)
            .await
            .expect("segment should insert");
        }
        pool
    }

    #[test]
    fn buckets_days_at_the_local_utc_offset() {
        block_on(async {
            // 18:40 UTC is 00:10 the NEXT day at IST (+330), and 20:40 the same
            // day at UTC. Same row, two different days depending on the offset.
            let pool =
                pool_with(&[("screen", "2026-08-03T18:40:00Z", "2026-08-03T18:45:00Z")]).await;

            let utc = query_day_coverage(&pool, 0).await.expect("query");
            assert_eq!(utc.len(), 1);
            assert_eq!(utc[0].day, "2026-08-03");
            assert_eq!(utc[0].hours, vec![18]);

            let ist = query_day_coverage(&pool, 330).await.expect("query");
            assert_eq!(ist.len(), 1);
            assert_eq!(ist[0].day, "2026-08-04", "IST rolls past local midnight");
            assert_eq!(ist[0].hours, vec![0]);

            // Negative offsets walk the other way: 18:40Z is 11:40 at PDT (-420).
            let pdt = query_day_coverage(&pool, -420).await.expect("query");
            assert_eq!(pdt[0].day, "2026-08-03");
            assert_eq!(pdt[0].hours, vec![11]);
        });
    }

    #[test]
    fn a_segment_crossing_local_midnight_belongs_to_the_day_it_started() {
        block_on(async {
            let pool = pool_with(&[
                ("screen", "2026-08-03T23:58:00Z", "2026-08-04T00:03:00Z"),
                ("screen", "2026-08-04T00:03:00Z", "2026-08-04T00:08:00Z"),
            ])
            .await;

            let days = query_day_coverage(&pool, 0).await.expect("query");
            assert_eq!(days.len(), 2);
            assert_eq!(days[0].day, "2026-08-03");
            assert_eq!(days[0].hours, vec![23]);
            assert_eq!(days[0].covered_ms, 5 * 60_000);
            assert_eq!(days[1].day, "2026-08-04");
            assert_eq!(days[1].hours, vec![0]);
        });
    }

    #[test]
    fn dst_transition_neither_drops_nor_duplicates_a_day() {
        block_on(async {
            // US spring-forward 2026-03-08: 09:59Z .. 10:01Z straddles the
            // instant local clocks jump 02:00 -> 03:00 in America/New_York.
            // Bucketing at a fixed offset (-300, EST) keeps both segments on the
            // same local day in consecutive hours — no 25-hour day, no gap.
            let pool = pool_with(&[
                ("screen", "2026-03-08T06:30:00Z", "2026-03-08T06:35:00Z"),
                ("screen", "2026-03-08T07:30:00Z", "2026-03-08T07:35:00Z"),
                ("screen", "2026-03-08T08:30:00Z", "2026-03-08T08:35:00Z"),
            ])
            .await;

            let days = query_day_coverage(&pool, -300).await.expect("query");
            assert_eq!(days.len(), 1, "one local day, not two");
            assert_eq!(days[0].day, "2026-03-08");
            assert_eq!(days[0].hours, vec![1, 2, 3]);
            assert_eq!(days[0].covered_ms, 15 * 60_000);

            // Re-bucketing the same rows at the post-transition offset (-240,
            // EDT) shifts every hour by one and still yields exactly one day.
            let edt = query_day_coverage(&pool, -240).await.expect("query");
            assert_eq!(edt.len(), 1);
            assert_eq!(edt[0].hours, vec![2, 3, 4]);
        });
    }

    #[test]
    fn empty_days_are_absent_and_an_empty_table_yields_nothing() {
        block_on(async {
            let empty = pool_with(&[]).await;
            assert!(query_day_coverage(&empty, 0)
                .await
                .expect("query")
                .is_empty());

            // Aug 3 and Aug 5 recorded; Aug 4 did not. The gap day must not
            // appear at all — its absence is what disables it in the menu.
            let pool = pool_with(&[
                ("screen", "2026-08-03T09:00:00Z", "2026-08-03T09:05:00Z"),
                ("screen", "2026-08-05T09:00:00Z", "2026-08-05T09:05:00Z"),
            ])
            .await;
            let days = query_day_coverage(&pool, 0).await.expect("query");
            assert_eq!(
                days.iter().map(|d| d.day.as_str()).collect::<Vec<_>>(),
                vec!["2026-08-03", "2026-08-05"]
            );
        });
    }

    #[test]
    fn concurrent_sources_do_not_multiply_covered_time() {
        block_on(async {
            // Screen + mic + system audio recording the same 5 minutes must read
            // as 5 minutes, not 15.
            let pool = pool_with(&[
                ("screen", "2026-08-03T09:00:00Z", "2026-08-03T09:05:00Z"),
                ("microphone", "2026-08-03T09:00:00Z", "2026-08-03T09:05:00Z"),
                (
                    "system_audio",
                    "2026-08-03T09:00:00Z",
                    "2026-08-03T09:05:00Z",
                ),
            ])
            .await;
            let days = query_day_coverage(&pool, 0).await.expect("query");
            assert_eq!(days[0].covered_ms, 5 * 60_000);
        });
    }

    #[test]
    fn an_hour_never_reads_as_more_than_an_hour() {
        block_on(async {
            // A pathological row (segments are capped at 5 min in practice) must
            // not make a day claim 30 hours of capture.
            let pool =
                pool_with(&[("screen", "2026-08-03T09:00:00Z", "2026-08-04T15:00:00Z")]).await;
            let days = query_day_coverage(&pool, 0).await.expect("query");
            assert_eq!(days[0].covered_ms, HOUR_MS);
        });
    }

    #[test]
    fn offset_modifier_formats_a_sqlite_modifier() {
        assert_eq!(offset_modifier(330), "+330 minutes");
        assert_eq!(offset_modifier(-420), "-420 minutes");
        assert_eq!(offset_modifier(0), "+0 minutes");
    }
}
