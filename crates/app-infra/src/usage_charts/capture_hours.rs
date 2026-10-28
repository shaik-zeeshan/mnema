//! The UTC hours that hold any screen capture: the date picker's density
//! source. It is exactly the set of non-empty `activity_heatmap` buckets
//! [`UsageChartsStore::usage_charts`] would return for the same range, without
//! loading every frame row and parsing its metadata snapshot to get it.

use sqlx::Row;

use super::{ms_to_rfc3339, rfc3339_to_ms, UsageChartsStore};
use crate::Result;

/// Distinct `(hour, snapshot)` pairs come off `frames_usage_window_idx`
/// (migration 0060) without touching a frame row; only those pairs probe the
/// snapshot table. The lock-screen test mirrors
/// `FrameMetadataSnapshot::is_lock_screen` (lock-screen frames are time away,
/// so the heatmap drops them); a snapshot that isn't valid JSON counts as
/// capture there too, since the Rust parse of it fails the same way.
/// `captured_at` is UTC RFC3339 TEXT, so its first 13 chars name the UTC hour.
pub(crate) const CAPTURE_HOURS_SQL: &str = "\
    SELECT DISTINCT hours.hour AS hour \
    FROM (SELECT DISTINCT substr(captured_at, 1, 13) AS hour, metadata_snapshot_id AS snapshot_id \
          FROM frames \
          WHERE captured_at >= ?1 AND captured_at < ?2) AS hours \
    LEFT JOIN frame_metadata_snapshots AS s ON s.id = hours.snapshot_id \
    WHERE CASE WHEN json_valid(s.snapshot_json) \
               THEN coalesce(trim(json_extract(s.snapshot_json, '$.appBundleId')) = 'com.apple.loginwindow', 0) \
                 OR coalesce(trim(json_extract(s.snapshot_json, '$.appName')) = 'loginwindow', 0) \
               ELSE 0 END = 0 \
    ORDER BY hour ASC";

impl UsageChartsStore {
    /// Start (unix ms) of every UTC hour in `[start_ms, end_ms)` holding a
    /// non-lock-screen screen frame, ascending.
    pub async fn capture_hours(&self, start_ms: i64, end_ms: i64) -> Result<Vec<i64>> {
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

                // Same buckets as the heatmap the picker used to read.
                let charts = store
                    .usage_charts(Some(hour("00")), Some(hour("04")))
                    .await
                    .unwrap();
                let heatmap: Vec<i64> = charts
                    .activity_heatmap
                    .iter()
                    .filter(|bucket| bucket.intensity_count > 0)
                    .map(|bucket| bucket.bucket_start_ms)
                    .collect();
                assert_eq!(hours, heatmap);
            });
    }
}
