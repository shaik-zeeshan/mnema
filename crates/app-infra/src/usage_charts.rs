//! Engine-free, counting-only usage aggregations (issue #104) — the "Overview
//! free tier" charts computed entirely from already-captured Search Context
//! data. No Reasoning Engine, no model call: just SQL + arithmetic over the
//! `frames` + `frame_metadata_snapshots` tables.
//!
//! Privacy / ownership: this stays inside `app-infra`'s SQLite ownership and
//! reads only the per-frame `captured_at` timestamp, `session_id`, and the
//! frame metadata snapshot's app/url labels — never frame images, OCR text, or
//! audio. The Tauri layer is a thin adapter over [`UsageChartsStore`].
//!
//! ## Time-accounting heuristic
//!
//! "Time on app" is **estimated**, not measured. Screen frames are exported at
//! a ~1s cadence ([`capture_screen::DEFAULT_SCREEN_FRAME_EXPORT_INTERVAL`]) but
//! OCR/equivalence dedupe means stored frames can be much sparser during a
//! stable window. So we attribute, to each frame's frontmost app, the gap to
//! the *next* frame in the same capture session — capped at
//! [`MAX_FRAME_GAP_MS`] so a long idle/sleep gap (or a session boundary)
//! between two stored frames does not count as hours. The cap (30s) is well
//! above the 1s export cadence and the typical dedupe sparsity, but far below
//! the 5-minute segment cap, so a genuinely active stretch accumulates while an
//! away-from-keyboard stretch contributes at most one cap per gap. The final
//! frame of each session contributes one capped "tail" of [`MAX_FRAME_GAP_MS`]
//! so a single-frame visit is not worth zero.
//!
//! ## Heatmap bucketing
//!
//! Frames are counted into UTC-hour-aligned buckets (`bucket_start_ms` = the
//! frame's `captured_at` floored to the hour). Buckets are sparse and ascending.
//! The frontend re-derives day and hour-of-day from `bucket_start_ms`.

use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use capture_types::{AppTransition, AppUsage, HeatmapBucket, SiteUsage, UsageChartsResponse};

use crate::{Result, FRAME_SUBJECT_TYPE};

/// Maximum gap (ms) attributed to a single frame's frontmost app. Caps idle /
/// sleep / cross-session gaps so they do not inflate "time on app". See the
/// module docs for the rationale.
pub const MAX_FRAME_GAP_MS: i64 = 30_000;

/// One hour in milliseconds (heatmap bucket width).
const HOUR_MS: i64 = 3_600_000;

/// Label used when a frame has no app name and no bundle id.
const UNKNOWN_APP: &str = "Unknown";

/// Read store for the engine-free Overview usage charts. Wraps the shared
/// SQLite pool (mirrors `SearchStore` / `UserContextStore`).
#[derive(Clone)]
pub struct UsageChartsStore {
    pool: SqlitePool,
}

/// A single frame row pulled for aggregation, already mapped to its app label
/// and (optional) domain.
struct FrameRow {
    session_id: String,
    captured_at_ms: i64,
    /// App display name (or bundle id, or `UNKNOWN_APP`).
    app: String,
    app_bundle_id: Option<String>,
    /// Host extracted from `browser_url`, when the frame carried a URL.
    domain: Option<String>,
}

/// Mutable accumulator for one (app) entry while scanning frames.
#[derive(Default)]
struct AppAccum {
    active_ms: i64,
    frame_count: i64,
    bundle_id: Option<String>,
}

/// Mutable accumulator for one (domain) entry while scanning frames.
#[derive(Default)]
struct SiteAccum {
    active_ms: i64,
    frame_count: i64,
}

impl UsageChartsStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Computes the full engine-free Overview aggregation for the half-open
    /// range `[start_ms, end_ms)`.
    ///
    /// `start_ms` / `end_ms` are unix-millis bounds; pass `None` for an
    /// open-ended side (the store fills the missing bound from the actual
    /// min/max captured frame so `(None, None)` covers the full retained
    /// history). The returned `range_*_ms` echo the resolved bounds.
    pub async fn usage_charts(
        &self,
        start_ms: Option<i64>,
        end_ms: Option<i64>,
    ) -> Result<UsageChartsResponse> {
        // Resolve the open-ended side(s) from the actual data extent so the
        // echoed range is meaningful and the heatmap/range labels are tight.
        //
        // The fetch end bound is EXCLUSIVE (`< end`, to match the frontend's
        // half-open `[start, end)` calendar windows). When the caller leaves the
        // end open, we resolve it from `data_max_ms` (the newest frame's
        // timestamp); an exclusive bound would then drop that very frame from
        // its own full-history range, so we nudge the auto-resolved end one
        // second past the max frame to keep it inclusive. The nudge is a whole
        // second (not 1ms) on purpose: `frames.captured_at` is second-precision
        // RFC3339 TEXT, and a fractional-second bound (`…00.001Z`) sorts BEFORE
        // a non-fractional one (`…00Z`) under TEXT comparison, which would
        // silently exclude the max frame again. An explicit caller-supplied
        // `end_ms` is used as-is (half-open).
        let (data_min_ms, data_max_ms) = self.captured_at_extent().await?;
        let range_start_ms = start_ms.or(data_min_ms).unwrap_or(0);
        let mut range_end_ms = match end_ms {
            Some(explicit) => explicit,
            None => data_max_ms.map(|m| m + 1_000).unwrap_or(0),
        };
        if range_end_ms < range_start_ms {
            range_end_ms = range_start_ms;
        }

        let frames = self
            .fetch_frames(range_start_ms, range_end_ms)
            .await?;

        let (time_per_app, time_per_site, app_transitions, activity_heatmap) =
            aggregate(&frames);

        Ok(UsageChartsResponse {
            range_start_ms,
            range_end_ms,
            time_per_app,
            time_per_site,
            app_transitions,
            activity_heatmap,
        })
    }

    /// Min / max `captured_at` over all frames, as unix-millis. Either side is
    /// `None` when there are no frames.
    async fn captured_at_extent(&self) -> Result<(Option<i64>, Option<i64>)> {
        let row = sqlx::query(
            "SELECT MIN(captured_at) AS min_at, MAX(captured_at) AS max_at FROM frames",
        )
        .fetch_one(self.pool())
        .await?;

        let min_at: Option<String> = row.get("min_at");
        let max_at: Option<String> = row.get("max_at");
        Ok((
            min_at.as_deref().and_then(rfc3339_to_ms),
            max_at.as_deref().and_then(rfc3339_to_ms),
        ))
    }

    /// Fetches frames in the half-open range `[start_ms, end_ms)` joined to
    /// their metadata snapshot, ordered by session then capture time (the order
    /// the gap / transition scan needs). `frames.captured_at` is RFC3339 TEXT,
    /// so the range is bound as RFC3339 and re-parsed to millis at the boundary.
    ///
    /// The end bound is EXCLUSIVE (`< end_ms`) so it matches the frontend's
    /// `[startMs, endMs)` calendar windows (where a Day's `endMs` is the start
    /// of the next day): a frame captured at exactly next-day midnight belongs
    /// to the next day's window, not both. An inclusive end bound double-counts
    /// that boundary frame across adjacent ranges.
    async fn fetch_frames(&self, start_ms: i64, end_ms: i64) -> Result<Vec<FrameRow>> {
        let start_rfc3339 = ms_to_rfc3339(start_ms);
        let end_rfc3339 = ms_to_rfc3339(end_ms);

        let rows = sqlx::query(
            "SELECT frames.session_id AS session_id, \
                    frames.captured_at AS captured_at, \
                    frame_metadata_snapshots.snapshot_json AS snapshot_json \
             FROM frames \
             LEFT JOIN frame_metadata_snapshots \
                ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
             WHERE frames.captured_at >= ?1 AND frames.captured_at < ?2 \
             ORDER BY frames.session_id ASC, frames.captured_at ASC, frames.id ASC",
        )
        .bind(&start_rfc3339)
        .bind(&end_rfc3339)
        .fetch_all(self.pool())
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let captured_at: String = row.get("captured_at");
            let Some(captured_at_ms) = rfc3339_to_ms(&captured_at) else {
                continue;
            };
            let snapshot_json: Option<String> = row.get("snapshot_json");
            let (app, app_bundle_id, domain) = labels_from_snapshot(snapshot_json.as_deref());
            out.push(FrameRow {
                session_id: row.get("session_id"),
                captured_at_ms,
                app,
                app_bundle_id,
                domain,
            });
        }
        Ok(out)
    }
}

/// The pure aggregation over an already-fetched, session-then-time-ordered
/// frame list. Split out so it is unit-testable without a DB.
#[allow(clippy::type_complexity)]
fn aggregate(
    frames: &[FrameRow],
) -> (
    Vec<AppUsage>,
    Vec<SiteUsage>,
    Vec<AppTransition>,
    Vec<HeatmapBucket>,
) {
    let mut app_accum: HashMap<String, AppAccum> = HashMap::new();
    let mut site_accum: HashMap<String, SiteAccum> = HashMap::new();
    let mut transitions: HashMap<(String, String), i64> = HashMap::new();
    let mut heatmap: HashMap<i64, i64> = HashMap::new();

    // `prev` tracks the previous frame to derive the capped gap + transition,
    // but only within the same session.
    let mut prev: Option<&FrameRow> = None;

    for frame in frames {
        // Heatmap: count this frame in its UTC-hour bucket.
        let bucket = floor_to_hour(frame.captured_at_ms);
        *heatmap.entry(bucket).or_insert(0) += 1;

        // Frame counts are per-frame regardless of gaps.
        let app_entry = app_accum.entry(frame.app.clone()).or_default();
        app_entry.frame_count += 1;
        if app_entry.bundle_id.is_none() {
            app_entry.bundle_id = frame.app_bundle_id.clone();
        }
        if let Some(domain) = &frame.domain {
            site_accum.entry(domain.clone()).or_default().frame_count += 1;
        }

        if let Some(previous) = prev {
            if previous.session_id == frame.session_id {
                // Same session: attribute the capped gap to the PREVIOUS
                // frame's app/domain (the app that was frontmost during the
                // gap), and record the focus transition.
                let gap = (frame.captured_at_ms - previous.captured_at_ms)
                    .clamp(0, MAX_FRAME_GAP_MS);
                app_accum
                    .entry(previous.app.clone())
                    .or_default()
                    .active_ms += gap;
                if let Some(domain) = &previous.domain {
                    site_accum.entry(domain.clone()).or_default().active_ms += gap;
                }
                if previous.app != frame.app {
                    *transitions
                        .entry((previous.app.clone(), frame.app.clone()))
                        .or_insert(0) += 1;
                }
            } else {
                // Session boundary: give the previous (final-in-session) frame
                // a single capped tail so a short/single-frame visit is not
                // worth zero active time.
                add_tail(&mut app_accum, &mut site_accum, previous);
            }
        }

        prev = Some(frame);
    }

    // Tail for the very last frame overall.
    if let Some(previous) = prev {
        add_tail(&mut app_accum, &mut site_accum, previous);
    }

    let mut time_per_app: Vec<AppUsage> = app_accum
        .into_iter()
        .map(|(app, acc)| AppUsage {
            app,
            app_bundle_id: acc.bundle_id,
            active_ms: acc.active_ms,
            frame_count: acc.frame_count,
        })
        .collect();
    time_per_app.sort_by(|a, b| {
        b.active_ms
            .cmp(&a.active_ms)
            .then_with(|| b.frame_count.cmp(&a.frame_count))
            .then_with(|| a.app.cmp(&b.app))
    });

    let mut time_per_site: Vec<SiteUsage> = site_accum
        .into_iter()
        .map(|(domain, acc)| SiteUsage {
            domain,
            active_ms: acc.active_ms,
            frame_count: acc.frame_count,
        })
        .collect();
    time_per_site.sort_by(|a, b| {
        b.active_ms
            .cmp(&a.active_ms)
            .then_with(|| b.frame_count.cmp(&a.frame_count))
            .then_with(|| a.domain.cmp(&b.domain))
    });

    let mut app_transitions: Vec<AppTransition> = transitions
        .into_iter()
        .map(|((from_app, to_app), count)| AppTransition {
            from_app,
            to_app,
            count,
        })
        .collect();
    app_transitions.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.from_app.cmp(&b.from_app))
            .then_with(|| a.to_app.cmp(&b.to_app))
    });

    let mut activity_heatmap: Vec<HeatmapBucket> = heatmap
        .into_iter()
        .map(|(bucket_start_ms, intensity_count)| HeatmapBucket {
            bucket_start_ms,
            intensity_count,
        })
        .collect();
    activity_heatmap.sort_by_key(|b| b.bucket_start_ms);

    (time_per_app, time_per_site, app_transitions, activity_heatmap)
}

/// Adds the single capped "tail" of active time for a session's final frame.
fn add_tail(
    app_accum: &mut HashMap<String, AppAccum>,
    site_accum: &mut HashMap<String, SiteAccum>,
    frame: &FrameRow,
) {
    app_accum.entry(frame.app.clone()).or_default().active_ms += MAX_FRAME_GAP_MS;
    if let Some(domain) = &frame.domain {
        site_accum.entry(domain.clone()).or_default().active_ms += MAX_FRAME_GAP_MS;
    }
}

/// Maps a frame metadata snapshot JSON to `(app_label, bundle_id, domain)`.
/// `app_label` prefers the app name, then the bundle id, then `UNKNOWN_APP`.
/// `domain` is the host of `browser_url` when present.
fn labels_from_snapshot(
    snapshot_json: Option<&str>,
) -> (String, Option<String>, Option<String>) {
    let Some(snapshot_json) = snapshot_json else {
        return (UNKNOWN_APP.to_string(), None, None);
    };
    let Ok(snapshot) =
        serde_json::from_str::<capture_metadata::FrameMetadataSnapshot>(snapshot_json)
    else {
        return (UNKNOWN_APP.to_string(), None, None);
    };

    let bundle_id = snapshot
        .app_bundle_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let app = snapshot
        .app_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| bundle_id.clone())
        .unwrap_or_else(|| UNKNOWN_APP.to_string());

    let domain = snapshot
        .browser_url
        .as_deref()
        .and_then(domain_from_url);

    (app, bundle_id, domain)
}

/// Extracts a lowercase host from a URL string without pulling in the `url`
/// crate: strips the scheme, any userinfo, the path/query/fragment, and the
/// port. Returns `None` for an empty/host-less value.
fn domain_from_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // Drop scheme.
    let after_scheme = match raw.split_once("://") {
        Some((_, rest)) => rest,
        None => raw,
    };
    // Authority ends at the first '/', '?', or '#'.
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // Drop userinfo (user:pass@host).
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    // Drop port. (IPv6 bracket forms are rare for browser_url; treat the part
    // before the last ':' that is not inside brackets as host.)
    let host = if host_port.starts_with('[') {
        // [::1]:8080 -> [::1]
        host_port
            .split_once(']')
            .map_or(host_port, |(h, _)| &host_port[..h.len() + 1])
    } else {
        host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
    };
    let host = host.trim().trim_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// Floors a unix-millis timestamp to its UTC hour start.
fn floor_to_hour(ms: i64) -> i64 {
    ms - ms.rem_euclid(HOUR_MS)
}

/// Converts unix milliseconds to an RFC3339 string for comparison against the
/// RFC3339 TEXT `frames.captured_at` column.
fn ms_to_rfc3339(ms: i64) -> String {
    let nanos = (ms as i128) * 1_000_000;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
        .unwrap_or_default()
}

/// Converts an RFC3339 TEXT timestamp to unix milliseconds; `None` on a parse
/// failure (the row is then skipped rather than poisoning the aggregation).
fn rfc3339_to_ms(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .map(|dt| (dt.unix_timestamp_nanos() / 1_000_000) as i64)
}

// Touch the constant so the module compiles cleanly even if a future refactor
// stops referencing `FRAME_SUBJECT_TYPE` directly; the frames table is the only
// subject here.
const _: &str = FRAME_SUBJECT_TYPE;

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

    fn frame(session: &str, ms: i64, app: &str, domain: Option<&str>) -> FrameRow {
        FrameRow {
            session_id: session.to_string(),
            captured_at_ms: ms,
            app: app.to_string(),
            app_bundle_id: None,
            domain: domain.map(str::to_string),
        }
    }

    #[test]
    fn domain_extraction_handles_common_url_shapes() {
        assert_eq!(
            domain_from_url("https://github.com/owner/repo?x=1#frag"),
            Some("github.com".to_string())
        );
        assert_eq!(
            domain_from_url("http://user:pass@Example.COM:8443/path"),
            Some("example.com".to_string())
        );
        assert_eq!(
            domain_from_url("docs.rs"),
            Some("docs.rs".to_string())
        );
        assert_eq!(domain_from_url(""), None);
        assert_eq!(domain_from_url("https://"), None);
    }

    #[test]
    fn floor_to_hour_aligns_to_utc_hour() {
        // 1970-01-01T01:30:00Z = 5_400_000 ms -> floors to 01:00 = 3_600_000.
        assert_eq!(floor_to_hour(5_400_000), 3_600_000);
        assert_eq!(floor_to_hour(0), 0);
    }

    #[test]
    fn aggregate_caps_gaps_and_counts_transitions() {
        let frames = vec![
            frame("s1", 0, "Editor", None),
            // 10s active in Editor.
            frame("s1", 10_000, "Editor", None),
            // 5-minute idle gap -> capped at 30s, attributed to Editor.
            frame("s1", 310_000, "Browser", Some("github.com")),
            // 1s in Browser then back to Editor.
            frame("s1", 311_000, "Editor", None),
        ];

        let (apps, sites, transitions, heatmap) = aggregate(&frames);

        let editor = apps.iter().find(|a| a.app == "Editor").expect("editor");
        // 10s + 30s capped gap + final tail 30s = 70s. (The 1s Browser->Editor
        // gap is attributed to Browser, and Editor's final frame gets a tail.)
        assert_eq!(editor.active_ms, 10_000 + 30_000 + 30_000);
        assert_eq!(editor.frame_count, 3);

        let browser = apps.iter().find(|a| a.app == "Browser").expect("browser");
        // 1s gap (Browser->Editor). Browser is not the last frame, so no tail.
        assert_eq!(browser.active_ms, 1_000);
        assert_eq!(browser.frame_count, 1);

        // Transitions: Editor->Browser once, Browser->Editor once.
        assert!(transitions
            .iter()
            .any(|t| t.from_app == "Editor" && t.to_app == "Browser" && t.count == 1));
        assert!(transitions
            .iter()
            .any(|t| t.from_app == "Browser" && t.to_app == "Editor" && t.count == 1));
        // No self-transition for the consecutive Editor frames.
        assert!(!transitions
            .iter()
            .any(|t| t.from_app == "Editor" && t.to_app == "Editor"));

        // Site: only the github.com frame.
        let gh = sites.iter().find(|s| s.domain == "github.com").expect("gh");
        assert_eq!(gh.frame_count, 1);
        // github.com's frame is followed by a 1s same-session gap -> 1s active.
        assert_eq!(gh.active_ms, 1_000);

        // Heatmap: all four frames fall in the 00:00 UTC hour bucket.
        assert_eq!(heatmap.len(), 1);
        assert_eq!(heatmap[0].bucket_start_ms, 0);
        assert_eq!(heatmap[0].intensity_count, 4);
    }

    #[test]
    fn aggregate_does_not_bridge_gaps_across_sessions() {
        let frames = vec![
            frame("s1", 0, "Editor", None),
            // Different session -> the s1 final frame gets a tail, NOT a gap to
            // the s2 frame.
            frame("s2", 1_000, "Editor", None),
        ];
        let (apps, _sites, transitions, _heatmap) = aggregate(&frames);
        let editor = apps.iter().find(|a| a.app == "Editor").expect("editor");
        // s1 tail (30s) + s2 tail (30s) = 60s. No cross-session gap or transition.
        assert_eq!(editor.active_ms, 60_000);
        assert!(transitions.is_empty());
    }

    /// End-to-end against an in-memory DB with the real `frames` +
    /// `frame_metadata_snapshots` shape, exercising the SQL and the RFC3339
    /// boundary conversion.
    #[test]
    fn usage_charts_query_runs_against_sqlite() {
        block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("in-memory db");

            sqlx::query(
                "CREATE TABLE frame_metadata_snapshots (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    normalized_hash TEXT NOT NULL,
                    snapshot_json TEXT NOT NULL
                )",
            )
            .execute(&pool)
            .await
            .expect("create snapshots");
            sqlx::query(
                "CREATE TABLE frames (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    captured_at TEXT NOT NULL,
                    metadata_snapshot_id INTEGER
                )",
            )
            .execute(&pool)
            .await
            .expect("create frames");

            // Two snapshots: Safari w/ url, and a plain app.
            sqlx::query(
                "INSERT INTO frame_metadata_snapshots (id, normalized_hash, snapshot_json) \
                 VALUES (1, 'h1', ?1), (2, 'h2', ?2)",
            )
            .bind(
                r#"{"appBundleId":"com.apple.Safari","appName":"Safari","windowTitle":"GH","browserUrl":"https://github.com/x/y"}"#,
            )
            .bind(r#"{"appBundleId":"com.foo.Editor","appName":"Editor"}"#)
            .execute(&pool)
            .await
            .expect("insert snapshots");

            sqlx::query(
                "INSERT INTO frames (session_id, file_path, captured_at, metadata_snapshot_id) VALUES \
                 ('s1', 'a.jpg', '2026-01-01T00:00:00Z', 2), \
                 ('s1', 'b.jpg', '2026-01-01T00:00:05Z', 1), \
                 ('s1', 'c.jpg', '2026-01-01T01:30:00Z', 2)",
            )
            .execute(&pool)
            .await
            .expect("insert frames");

            let store = UsageChartsStore::new(pool);

            // Full-history (None, None) covers all three frames.
            let resp = store.usage_charts(None, None).await.expect("charts");
            assert!(resp.range_start_ms <= resp.range_end_ms);
            // Editor + Safari both present.
            assert!(resp.time_per_app.iter().any(|a| a.app == "Editor"));
            let safari = resp
                .time_per_app
                .iter()
                .find(|a| a.app == "Safari")
                .expect("safari");
            assert_eq!(safari.app_bundle_id.as_deref(), Some("com.apple.Safari"));
            // Site exists because Safari frame carried a URL.
            assert!(resp
                .time_per_site
                .iter()
                .any(|s| s.domain == "github.com"));
            // Heatmap spans two hours (00:00 and 01:00).
            assert_eq!(resp.activity_heatmap.len(), 2);

            // Narrow range to just the first hour -> excludes the 01:30 frame.
            let start = rfc3339_to_ms("2026-01-01T00:00:00Z").unwrap();
            let end = rfc3339_to_ms("2026-01-01T00:59:59Z").unwrap();
            let narrowed = store
                .usage_charts(Some(start), Some(end))
                .await
                .expect("narrowed");
            assert_eq!(narrowed.activity_heatmap.len(), 1);

            // Half-open end bound: a request whose `end_ms` is EXACTLY the
            // boundary of the next frame must exclude that frame (it belongs to
            // the next window). The first frame is at 00:00:00; requesting
            // [00:00:00, 00:00:05) must include only the 00:00:00 frame, not the
            // 00:00:05 one — proving `< end_ms`, not `<= end_ms`. This is the
            // Day/Week boundary contract: a Day's `endMs` is the next day's
            // start, and a frame at that instant must not be double-counted.
            let half_open_start = rfc3339_to_ms("2026-01-01T00:00:00Z").unwrap();
            let half_open_end = rfc3339_to_ms("2026-01-01T00:00:05Z").unwrap();
            let half_open = store
                .usage_charts(Some(half_open_start), Some(half_open_end))
                .await
                .expect("half-open");
            // Only the 00:00:00 frame is in [00:00:00, 00:00:05): Editor, 1 frame.
            let editor = half_open
                .time_per_app
                .iter()
                .find(|a| a.app == "Editor")
                .expect("editor in half-open window");
            assert_eq!(
                editor.frame_count, 1,
                "the 00:00:05 frame must be excluded by the exclusive end bound"
            );
            assert!(
                half_open.time_per_app.iter().all(|a| a.app != "Safari"),
                "the 00:00:05 Safari frame sits on the exclusive end bound and must be excluded"
            );
        });
    }
}
