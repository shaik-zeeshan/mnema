//! The local hours that hold any screen capture: the date picker's density
//! source. With a whole-hour offset it is exactly the set of non-empty
//! `activity_heatmap` buckets [`UsageChartsStore::usage_charts`] would return
//! for the same range, without loading every frame row and parsing its
//! metadata snapshot to get it.

use sqlx::Row;

use super::{ms_to_rfc3339, UsageChartsStore};
use crate::{AppInfraError, Result};

/// A skip-scan of the frames' `captured_at` indexes: each step seeks the first
/// frame at or after the next local hour, so the walk costs a few page reads per
/// captured hour instead of one index entry per frame in the window. `?3` is the
/// UTC offset in seconds; an hour's start is the frame's minute floored to a
/// local hour, carried as unix seconds. Bounds compare as 16-char
/// `YYYY-MM-DDTHH:MM` prefixes: `captured_at` is UTC RFC3339 TEXT, and a
/// prefix sorts before every timestamp that starts with it.
///
/// An hour counts unless every frame in it is a lock screen, mirroring
/// `FrameMetadataSnapshot::is_lock_screen` (lock-screen frames are time away,
/// so the heatmap drops them); the `EXISTS` stops at the hour's first other
/// frame. A missing snapshot, or one that isn't valid JSON, counts as capture
/// there too, since the Rust parse of it fails the same way.
/// The lock-screen test approximates `is_lock_screen` on purpose: SQL `trim()`
/// strips only spaces where Rust's `trim()` strips all whitespace, which only
/// matters for an id padded with tabs or newlines that no capture produces.
pub(crate) const CAPTURE_HOURS_SQL: &str = "\
    WITH RECURSIVE hours(start) AS ( \
        SELECT (SELECT (CAST(strftime('%s', substr(captured_at, 1, 16)) AS INTEGER) + ?3) \
                       / 3600 * 3600 - ?3 \
                FROM frames \
                WHERE captured_at >= ?1 AND captured_at < ?2 \
                ORDER BY captured_at LIMIT 1) \
        UNION ALL \
        SELECT (SELECT (CAST(strftime('%s', substr(captured_at, 1, 16)) AS INTEGER) + ?3) \
                       / 3600 * 3600 - ?3 \
                FROM frames \
                WHERE captured_at >= strftime('%Y-%m-%dT%H:%M', hours.start + 3600, 'unixepoch') \
                  AND captured_at < ?2 \
                ORDER BY captured_at LIMIT 1) \
        FROM hours WHERE hours.start IS NOT NULL) \
    SELECT start FROM hours \
    WHERE start IS NOT NULL AND EXISTS ( \
        SELECT 1 FROM frames AS f \
        LEFT JOIN frame_metadata_snapshots AS s ON s.id = f.metadata_snapshot_id \
        WHERE f.captured_at >= max(?1, strftime('%Y-%m-%dT%H:%M', hours.start, 'unixepoch')) \
          AND f.captured_at < min(?2, strftime('%Y-%m-%dT%H:%M', hours.start + 3600, 'unixepoch')) \
          AND CASE WHEN json_valid(s.snapshot_json) \
                   THEN coalesce(trim(json_extract(s.snapshot_json, '$.appBundleId')) = 'com.apple.loginwindow', 0) \
                     OR coalesce(trim(json_extract(s.snapshot_json, '$.appName')) = 'loginwindow', 0) \
                   ELSE 0 END = 0) \
    ORDER BY start ASC";

/// The widest window [`UsageChartsStore::capture_hours`] serves. The picker asks
/// for at most ~121 days; a year plus slack keeps any caller off a walk of the
/// whole index (an out-of-range bound formats as "", which compares below
/// every timestamp).
pub(crate) const CAPTURE_HOURS_MAX_SPAN_MS: i64 = 400 * 86_400_000;

impl UsageChartsStore {
    /// Start (unix ms) of every local hour in `[start_ms, end_ms)` holding a
    /// non-lock-screen screen frame, ascending. Hours are aligned to
    /// `offset_minutes` east of UTC (IST = 330), so a half-hour zone's midnight
    /// opens a bucket; only the offset's minutes within the hour matter, so one
    /// offset serves a window that crosses a whole-hour DST change. Refuses a
    /// reversed window or one wider than [`CAPTURE_HOURS_MAX_SPAN_MS`].
    pub async fn capture_hours(
        &self,
        start_ms: i64,
        end_ms: i64,
        offset_minutes: i32,
    ) -> Result<Vec<i64>> {
        match end_ms.checked_sub(start_ms) {
            Some(span) if (0..=CAPTURE_HOURS_MAX_SPAN_MS).contains(&span) => {}
            _ => {
                return Err(AppInfraError::InvalidTimeRange(format!(
                    "capture hours window must run forward and span at most \
                     {} days, got [{start_ms}, {end_ms})",
                    CAPTURE_HOURS_MAX_SPAN_MS / 86_400_000
                )))
            }
        }
        let rows = sqlx::query(CAPTURE_HOURS_SQL)
            .bind(ms_to_rfc3339(start_ms))
            .bind(ms_to_rfc3339(end_ms))
            .bind(i64::from(offset_minutes) * 60)
            .fetch_all(self.pool())
            .await?;
        Ok(rows
            .iter()
            .map(|row| row.get::<i64, _>("start") * 1000)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::CaptureDb;
    use crate::usage_charts::rfc3339_to_ms;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn capture_hours_match_the_usage_heatmap_buckets() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(async {
                let pool = SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect("sqlite::memory:")
                    .await
                    .expect("in-memory db");
                for ddl in [
                    "CREATE TABLE frame_metadata_snapshots (id INTEGER PRIMARY KEY, \
                     normalized_hash TEXT NOT NULL, snapshot_json TEXT NOT NULL)",
                    "CREATE TABLE frames (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                     session_id TEXT NOT NULL, file_path TEXT NOT NULL, \
                     captured_at TEXT NOT NULL, metadata_snapshot_id INTEGER)",
                    "INSERT INTO frame_metadata_snapshots VALUES \
                     (1, 'h1', '{\"appBundleId\":\"com.foo.Editor\",\"appName\":\"Editor\"}'), \
                     (2, 'h2', '{\"appBundleId\":\"com.apple.loginwindow\",\"appName\":\"loginwindow\"}'), \
                     (3, 'h3', 'not json')",
                    // 00h: app + lock screen; 01h: lock screen only; 02h: no
                    // snapshot; 03h: unparseable snapshot; 04h: the excluded
                    // end bound itself.
                    "INSERT INTO frames (session_id, file_path, captured_at, metadata_snapshot_id) VALUES \
                     ('s', 'a', '2026-01-01T00:10:00Z', 1), \
                     ('s', 'b', '2026-01-01T00:20:00Z', 2), \
                     ('s', 'c', '2026-01-01T00:59:59Z', 1), \
                     ('s', 'd', '2026-01-01T01:30:00Z', 2), \
                     ('s', 'e', '2026-01-01T02:00:00Z', NULL), \
                     ('s', 'f', '2026-01-01T03:00:00Z', 3), \
                     ('s', 'g', '2026-01-01T04:00:00Z', 1)",
                ] {
                    sqlx::query(ddl).execute(&pool).await.expect("fixture");
                }
                let store = UsageChartsStore::new(CaptureDb::single(pool));
                let hour = |h: &str| rfc3339_to_ms(&format!("2026-01-01T{h}:00:00Z")).unwrap();

                let hours = store.capture_hours(hour("00"), hour("04"), 0).await.unwrap();
                assert_eq!(hours, vec![hour("00"), hour("02"), hour("03")]);

                // Same buckets as the heatmap the picker used to read, including
                // windows whose bounds fall mid-hour: from 00:15 only the 00:20
                // lock screen and the 00:59:59 app frame are in; from 00:21 only
                // the app frame; up to 00:59 neither.
                let minute = |m: i64| hour("00") + m * 60_000;
                for (start, end) in [
                    (hour("00"), hour("04")),
                    (minute(15), hour("04")),
                    (minute(21), hour("03")),
                    (hour("00"), minute(59)),
                    (hour("01"), hour("04")),
                ] {
                    let charts = store.usage_charts(Some(start), Some(end)).await.unwrap();
                    let heatmap: Vec<i64> = charts
                        .activity_heatmap
                        .iter()
                        .filter(|bucket| bucket.intensity_count > 0)
                        .map(|bucket| bucket.bucket_start_ms)
                        .collect();
                    assert_eq!(
                        store.capture_hours(start, end, 0).await.unwrap(),
                        heatmap,
                        "window [{start}, {end})"
                    );
                }
            });
    }

    #[test]
    fn a_half_hour_offset_buckets_by_local_hour() {
        // IST (+5:30): 18:40 UTC is 00:10 local on Jan 2, so its bucket is local
        // midnight (18:30 UTC), not the UTC hour from 23:30 local on Jan 1.
        // 18:20 UTC (23:50 local) is a lock screen, so the bucket before is empty.
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(async {
                let pool = SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect("sqlite::memory:")
                    .await
                    .expect("in-memory db");
                for ddl in [
                    "CREATE TABLE frame_metadata_snapshots (id INTEGER PRIMARY KEY, \
                     snapshot_json TEXT NOT NULL)",
                    "CREATE TABLE frames (id INTEGER PRIMARY KEY, captured_at TEXT NOT NULL, \
                     metadata_snapshot_id INTEGER)",
                    "INSERT INTO frame_metadata_snapshots VALUES \
                     (1, '{\"appBundleId\":\"com.foo.Editor\"}'), \
                     (2, '{\"appBundleId\":\"com.apple.loginwindow\"}')",
                    "INSERT INTO frames (captured_at, metadata_snapshot_id) VALUES \
                     ('2026-01-01T18:20:00Z', 2), ('2026-01-01T18:40:00Z', 1), \
                     ('2026-01-01T20:29:59Z', 1)",
                ] {
                    sqlx::query(ddl).execute(&pool).await.expect("fixture");
                }
                let store = UsageChartsStore::new(CaptureDb::single(pool));
                let at = |t: &str| rfc3339_to_ms(&format!("2026-01-01T{t}:00Z")).unwrap();

                assert_eq!(
                    store
                        .capture_hours(at("17:30"), at("21:30"), 330)
                        .await
                        .unwrap(),
                    vec![at("18:30"), at("19:30")]
                );
                // A window starting mid-hour still names the hour's local start.
                assert_eq!(
                    store
                        .capture_hours(at("18:35"), at("19:00"), 330)
                        .await
                        .unwrap(),
                    vec![at("18:30")]
                );
                // A negative half-hour offset (-9:30) aligns the same way.
                assert_eq!(
                    store
                        .capture_hours(at("17:30"), at("21:30"), -570)
                        .await
                        .unwrap(),
                    vec![at("18:30"), at("19:30")]
                );
            });
    }

    #[test]
    fn capture_hours_refuses_reversed_and_oversized_windows() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(async {
                let pool = SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect("sqlite::memory:")
                    .await
                    .expect("in-memory db");
                sqlx::query(
                    "CREATE TABLE frames (id INTEGER PRIMARY KEY, captured_at TEXT NOT NULL, \
                     metadata_snapshot_id INTEGER)",
                )
                .execute(&pool)
                .await
                .expect("fixture");
                sqlx::query(
                    "CREATE TABLE frame_metadata_snapshots (id INTEGER PRIMARY KEY, \
                     snapshot_json TEXT NOT NULL)",
                )
                .execute(&pool)
                .await
                .expect("fixture");
                let store = UsageChartsStore::new(CaptureDb::single(pool));

                for (start, end) in [
                    (i64::MIN, i64::MAX),
                    (1_000, 0),
                    (0, CAPTURE_HOURS_MAX_SPAN_MS + 1),
                ] {
                    assert!(
                        matches!(
                            store.capture_hours(start, end, 0).await,
                            Err(AppInfraError::InvalidTimeRange(_))
                        ),
                        "window [{start}, {end}) must be refused as a bad time range"
                    );
                }
                assert!(store.capture_hours(0, 0, 0).await.unwrap().is_empty());
                assert!(store
                    .capture_hours(0, CAPTURE_HOURS_MAX_SPAN_MS, 0)
                    .await
                    .is_ok());
            });
    }
}
