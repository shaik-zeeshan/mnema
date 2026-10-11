//! Raw-capture time bounds for the derivation worker's cursors: the history
//! floor and the nearest capture on either side of an empty window.
//!
//! All three read RAW capture existence (`frames.captured_at`,
//! `audio_segments.started_at`, both legacy RFC3339 TEXT) — NOT OCR or
//! transcription completeness — so a cursor never jumps past frames whose OCR
//! is still pending. Each table is read independently and parsed at the
//! boundary exactly as `read_capture_window` does; a parse failure falls
//! through to the other source.

use sqlx::Row;

use super::capture_source::ms_to_rfc3339;
use super::store::{rfc3339_text_to_ms, UserContextStore};
use crate::Result;

impl UserContextStore {
    /// The earliest capture across all history — the floor **go-deeper**
    /// backfill walks toward. `None` when there are no captures at all.
    pub async fn earliest_capture_at_ms(&self) -> Result<Option<i64>> {
        self.raw_capture_bound("MIN", "", None).await
    }

    /// The earliest capture **at or after** `after_ms`, or `None`. The forward
    /// worker jumps its cursor across an empty gap with this in one step
    /// instead of crawling 30-minute windows.
    pub async fn next_raw_capture_at_ms(&self, after_ms: i64) -> Result<Option<i64>> {
        self.raw_capture_bound("MIN", ">=", Some(after_ms)).await
    }

    /// The latest capture **strictly before** `before_ms`, or `None`: the
    /// backfill twin of [`Self::next_raw_capture_at_ms`], so a backward walk
    /// leaps an empty night (or the empty days below the first capture) in one
    /// step.
    pub async fn prev_raw_capture_before_ms(&self, before_ms: i64) -> Result<Option<i64>> {
        self.raw_capture_bound("MAX", "<", Some(before_ms)).await
    }

    /// `aggregate` (`MIN`/`MAX`) of both capture tables, optionally filtered by
    /// `column <op> at_ms`; the smaller (MIN) or larger (MAX) of the two wins.
    async fn raw_capture_bound(
        &self,
        aggregate: &str,
        op: &str,
        at_ms: Option<i64>,
    ) -> Result<Option<i64>> {
        let mut bounds = Vec::with_capacity(2);
        for (table, column) in [("frames", "captured_at"), ("audio_segments", "started_at")] {
            let filter = at_ms.map_or(String::new(), |_| format!(" WHERE {column} {op} ?1"));
            let sql = format!("SELECT {aggregate}({column}) AS m FROM {table}{filter}");
            let mut query = sqlx::query(&sql);
            if let Some(at_ms) = at_ms {
                query = query.bind(ms_to_rfc3339(at_ms));
            }
            let text: Option<String> = query.fetch_one(self.db.read()).await?.get("m");
            bounds.extend(text.as_deref().and_then(rfc3339_text_to_ms));
        }
        Ok(if aggregate == "MAX" {
            bounds.into_iter().max()
        } else {
            bounds.into_iter().min()
        })
    }
}
