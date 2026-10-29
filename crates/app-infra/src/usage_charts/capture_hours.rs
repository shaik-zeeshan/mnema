//! The UTC hours that hold any screen capture: the date picker's density
//! source. It is exactly the set of non-empty `activity_heatmap` buckets
//! [`UsageChartsStore::usage_charts`] would return for the same range, without
//! loading every frame row and parsing its metadata snapshot to get it.

use sqlx::Row;

use super::{ms_to_rfc3339, rfc3339_to_ms, UsageChartsStore};
use crate::{AppInfraError, Result};

/// A skip-scan of the frames' `captured_at` indexes: each step seeks the first
/// frame at or after the next UTC hour, so the walk costs a few page reads per
/// captured hour instead of one index entry per frame in the window. `hour || ';'`
/// is the next hour's lower bound: `captured_at` is UTC RFC3339 TEXT, its first
/// 13 chars name the hour, and `;` sorts just past the `:` that follows them.
///
/// An hour counts unless every frame in it is a lock screen, mirroring
/// `FrameMetadataSnapshot::is_lock_screen` (lock-screen frames are time away,
/// so the heatmap drops them); the `EXISTS` stops at the hour's first other
/// frame. A missing snapshot, or one that isn't valid JSON, counts as capture
/// there too, since the Rust parse of it fails the same way.
pub(crate) const CAPTURE_HOURS_SQL: &str = "\
    WITH RECURSIVE hours(hour) AS ( \
        SELECT substr((SELECT captured_at FROM frames \
                       WHERE captured_at >= ?1 AND captured_at < ?2 \
                       ORDER BY captured_at LIMIT 1), 1, 13) \
        UNION ALL \
        SELECT substr((SELECT captured_at FROM frames \
                       WHERE captured_at >= hours.hour || ';' AND captured_at < ?2 \
                       ORDER BY captured_at LIMIT 1), 1, 13) \
        FROM hours WHERE hours.hour IS NOT NULL) \
    SELECT hour FROM hours \
    WHERE hour IS NOT NULL AND EXISTS ( \
        SELECT 1 FROM frames AS f \
        LEFT JOIN frame_metadata_snapshots AS s ON s.id = f.metadata_snapshot_id \
        WHERE f.captured_at >= max(?1, hours.hour) AND f.captured_at < min(?2, hours.hour || ';') \
          AND CASE WHEN json_valid(s.snapshot_json) \
                   THEN coalesce(trim(json_extract(s.snapshot_json, '$.appBundleId')) = 'com.apple.loginwindow', 0) \
                     OR coalesce(trim(json_extract(s.snapshot_json, '$.appName')) = 'loginwindow', 0) \
                   ELSE 0 END = 0) \
    ORDER BY hour ASC";

/// The widest window [`UsageChartsStore::capture_hours`] serves. The picker asks
/// for at most ~121 days; a year plus slack keeps any caller off a walk of the
/// whole index (an out-of-range bound formats as "", which compares below
/// every timestamp).
pub(crate) const CAPTURE_HOURS_MAX_SPAN_MS: i64 = 400 * 86_400_000;

impl UsageChartsStore {
    /// Start (unix ms) of every UTC hour in `[start_ms, end_ms)` holding a
    /// non-lock-screen screen frame, ascending. Refuses a reversed window or one
    /// wider than [`CAPTURE_HOURS_MAX_SPAN_MS`].
    pub async fn capture_hours(&self, start_ms: i64, end_ms: i64) -> Result<Vec<i64>> {
        match end_ms.checked_sub(start_ms) {
            Some(span) if (0..=CAPTURE_HOURS_MAX_SPAN_MS).contains(&span) => {}
            _ => {
                return Err(AppInfraError::InvalidSearchRequest(format!(
                    "capture hours window must run forward and span at most \
                     {} days, got [{start_ms}, {end_ms})",
                    CAPTURE_HOURS_MAX_SPAN_MS / 86_400_000
                )))
            }
        }
        let rows = sqlx::query(CAPTURE_HOURS_SQL)
            .bind(ms_to_rfc3339(start_ms))
            .bind(ms_to_rfc3339(end_ms))
            .fetch_all(self.pool())
            .await?;
        Ok(rows
            .iter()
            .filter_map(|row| rfc3339_to_ms(&format!("{}:00:00Z", row.get::<String, _>("hour"))))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::CaptureDb;
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

                let hours = store.capture_hours(hour("00"), hour("04")).await.unwrap();
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
                        store.capture_hours(start, end).await.unwrap(),
                        heatmap,
                        "window [{start}, {end})"
                    );
                }
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
                        store.capture_hours(start, end).await.is_err(),
                        "window [{start}, {end}) must be refused"
                    );
                }
                assert!(store.capture_hours(0, 0).await.unwrap().is_empty());
                assert!(store
                    .capture_hours(0, CAPTURE_HOURS_MAX_SPAN_MS)
                    .await
                    .is_ok());
            });
    }
}
