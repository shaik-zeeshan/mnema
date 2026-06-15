//! Usage-chart DTOs (issue #104) — the engine-free, counting-only "Overview
//! free tier" aggregations computed from already-captured Search Context data.
//!
//! These carry no logic; the aggregation queries live in
//! `crates/app-infra/src/usage_charts.rs` and the thin Tauri adapter lives in
//! `apps/desktop/src-tauri/src/usage_charts.rs`.
//!
//! Conventions (matching the rest of `capture-types`): structs use
//! `#[serde(rename_all = "camelCase")]`. All timestamps are `i64` unix
//! milliseconds. Counts and durations are `i64`. These derive `Eq` because no
//! field is a float.

use serde::{Deserialize, Serialize};

/// Request for [`UsageChartsResponse`]. Both bounds are optional unix-millis;
/// `None` means "open-ended" on that side, so `{}` covers the full retained
/// history. The store clamps `start_ms <= end_ms`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageChartsRequest {
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
}

/// Total active ("time on app") plus raw frame count per frontmost application
/// over the range, highest `active_ms` first.
///
/// `active_ms` is the **estimated** time on the app: the sum of gaps between
/// consecutive frames of the same frontmost app within a single capture
/// session, with each gap capped (so a long idle/sleep gap between two frames
/// does not count as hours). See the store for the cap value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    /// App display name when known, else the bundle id, else `"Unknown"`.
    pub app: String,
    /// Bundle id when known (e.g. `com.apple.Safari`), else `None`.
    pub app_bundle_id: Option<String>,
    pub active_ms: i64,
    pub frame_count: i64,
}

/// Domain-level time + frame count, ONLY where browser URL metadata exists on
/// the frame. Empty when no captured frame carried a `browserUrl`. Highest
/// `active_ms` first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SiteUsage {
    /// Host/domain extracted from the captured URL (e.g. `github.com`).
    pub domain: String,
    pub active_ms: i64,
    pub frame_count: i64,
}

/// One directed edge of the app-interaction graph: how often frontmost focus
/// moved from `from_app` to `to_app` (adjacent frame transitions within a
/// session). Highest `count` first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppTransition {
    pub from_app: String,
    pub to_app: String,
    pub count: i64,
}

/// One hour-aligned bucket of the activity heatmap. `bucket_start_ms` is the
/// UTC hour start (unix-millis floored to the hour); `intensity_count` is the
/// number of captured frames in that hour. The frontend can re-derive
/// day-of-range and hour-of-day from `bucket_start_ms` to render either a
/// per-day strip or a day×hour grid. Buckets are ascending and sparse (only
/// hours with at least one frame appear).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapBucket {
    pub bucket_start_ms: i64,
    pub intensity_count: i64,
}

/// The full engine-free Overview aggregation for a range.
///
/// `range_start_ms` / `range_end_ms` echo the resolved (default-filled) range
/// the store actually queried, so the frontend can label the window even when
/// the request left a bound open.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageChartsResponse {
    pub range_start_ms: i64,
    pub range_end_ms: i64,
    pub time_per_app: Vec<AppUsage>,
    /// Always present; empty when no captured frame carried browser URL
    /// metadata over the range.
    pub time_per_site: Vec<SiteUsage>,
    pub app_transitions: Vec<AppTransition>,
    pub activity_heatmap: Vec<HeatmapBucket>,
}
