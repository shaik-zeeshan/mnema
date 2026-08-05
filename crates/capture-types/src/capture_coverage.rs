//! Per-day capture coverage DTO — what the timeline's jump menu renders for
//! each day row and each month-grid cell (round-4 decision G6).
//!
//! No logic here: the single GROUP BY that produces these rows lives in
//! `crates/app-infra/src/capture_coverage.rs` and the thin Tauri adapter in
//! `apps/desktop/src-tauri/src/capture_coverage.rs`.
//!
//! Conventions match the rest of `capture-types`: `#[serde(rename_all =
//! "camelCase")]`, durations as `i64` milliseconds.

use serde::{Deserialize, Serialize};

/// One local calendar day that holds at least one capture segment. Days with no
/// recording are simply absent — that absence is what disables them in the jump
/// menu (G6: "you can never land on an empty day").
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DayCoverage {
    /// `YYYY-MM-DD` in the user's local UTC offset (the frontend-stamped
    /// `user_context.local_offset_minutes`, UTC when never stamped).
    pub day: String,
    /// Wall-clock milliseconds of capture on that day, per-hour capped at one
    /// hour and de-overlapped across the three capture sources. An estimate:
    /// see the store's module docs.
    pub covered_ms: i64,
    /// Ascending local hours (`0..=23`) that hold any capture. Drives the day
    /// row's coverage bar.
    pub hours: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_coverage_round_trips_camel_case() {
        let day = DayCoverage {
            day: "2026-08-03".to_string(),
            covered_ms: 24_120_000,
            hours: vec![9, 10, 11, 14],
        };
        let json = serde_json::to_string(&day).expect("serialize");
        assert!(json.contains("\"coveredMs\":24120000"), "{json}");
        assert_eq!(
            serde_json::from_str::<DayCoverage>(&json).expect("deserialize"),
            day
        );
    }
}
