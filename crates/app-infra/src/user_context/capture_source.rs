//! Capture-window reader for the User Context derivation worker.
//!
//! Assembles the already-redacted OCR / transcript **text** (plus best-effort
//! Search Context app/url labels) for captures inside a time window, so the
//! derivation worker can hand a window to the Reasoning Engine.
//!
//! Privacy invariant (spec §7): this reader only ever reads
//! `processing_results.result_text` — the already-redacted OCR / transcript
//! text — and the Search Context labels. It never reads frame images or audio
//! files. Only this redacted text is ever sent to a cloud engine.

use sqlx::Row;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    Result, AUDIO_SEGMENT_SUBJECT_TYPE, AUDIO_TRANSCRIPTION_PROCESSOR, FRAME_SUBJECT_TYPE,
    OCR_PROCESSOR,
};

use super::store::UserContextStore;

/// One redacted-text capture inside a [`CaptureWindow`], tagged with its raw
/// subject identity so the derivation worker can map an Activity's evidence
/// back to frames / audio segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureWindowItem {
    /// `"frame"` | `"audio_segment"`.
    pub subject_type: String,
    pub subject_id: i64,
    pub captured_at_ms: i64,
    /// OCR `result_text` OR transcription `result_text` (already redacted).
    pub text: String,
    /// Search Context app/window label, if available (frames only, best-effort).
    pub app_label: Option<String>,
    /// Search Context URL, if available (frames only, best-effort).
    pub url: Option<String>,
}

/// The redacted-text captures inside `[start_ms, end_ms]`, time-ordered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureWindow {
    pub items: Vec<CaptureWindowItem>,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl UserContextStore {
    /// Reads the redacted OCR / transcript text for captures inside
    /// `[start_ms, end_ms]`, capped at `max_items`, ordered by
    /// `captured_at_ms` ascending.
    ///
    /// Frames are joined to their latest `processor='ocr'` processing result
    /// (non-empty `result_text`); audio segments overlapping the window are
    /// joined to their latest `processor='audio_transcription'` result. Search
    /// Context app/url labels are pulled best-effort from the frame metadata
    /// snapshot.
    pub async fn read_capture_window(
        &self,
        start_ms: i64,
        end_ms: i64,
        max_items: i64,
    ) -> Result<CaptureWindow> {
        let start_rfc3339 = ms_to_rfc3339(start_ms);
        let end_rfc3339 = ms_to_rfc3339(end_ms);

        let mut items: Vec<CaptureWindowItem> = Vec::new();

        // --- Frames + latest OCR result + best-effort Search Context ---------
        //
        // Latest-per-frame OCR result via a MAX(id) self-join (mirrors the
        // pattern in `search.rs`). `frames.captured_at` is RFC3339 TEXT; filter
        // on the RFC3339 range and convert to millis at the boundary.
        let frame_rows = sqlx::query(
            "SELECT frames.id AS subject_id, \
                    frames.captured_at AS captured_at, \
                    processing_results.result_text AS result_text, \
                    frame_metadata_snapshots.snapshot_json AS snapshot_json \
             FROM frames \
             JOIN (\
                SELECT subject_id, MAX(id) AS id \
                FROM processing_results \
                WHERE subject_type = ?1 AND processor = ?2 \
                GROUP BY subject_id\
             ) latest_ocr ON latest_ocr.subject_id = frames.id \
             JOIN processing_results ON processing_results.id = latest_ocr.id \
             LEFT JOIN frame_metadata_snapshots \
                ON frame_metadata_snapshots.id = frames.metadata_snapshot_id \
             WHERE frames.captured_at >= ?3 AND frames.captured_at <= ?4 \
               AND LENGTH(TRIM(COALESCE(processing_results.result_text, ''))) > 0 \
             ORDER BY frames.captured_at ASC, frames.id ASC \
             LIMIT ?5",
        )
        .bind(FRAME_SUBJECT_TYPE)
        .bind(OCR_PROCESSOR)
        .bind(&start_rfc3339)
        .bind(&end_rfc3339)
        .bind(max_items)
        .fetch_all(self.pool())
        .await?;

        for row in frame_rows {
            let captured_at: String = row.get("captured_at");
            let Some(captured_at_ms) = rfc3339_to_ms(&captured_at) else {
                continue;
            };
            let text: String = row.get("result_text");
            let snapshot_json: Option<String> = row.get("snapshot_json");
            let (app_label, url) = search_context_from_snapshot(snapshot_json.as_deref());

            items.push(CaptureWindowItem {
                subject_type: FRAME_SUBJECT_TYPE.to_string(),
                subject_id: row.get("subject_id"),
                captured_at_ms,
                text,
                app_label,
                url,
            });
        }

        // --- Audio segments overlapping the window + latest transcription ----
        //
        // `audio_segments.started_at` / `ended_at` are RFC3339 TEXT. A segment
        // overlaps the window when it starts at-or-before the window end and
        // ends at-or-after the window start.
        let audio_rows = sqlx::query(
            "SELECT audio_segments.id AS subject_id, \
                    audio_segments.started_at AS started_at, \
                    processing_results.result_text AS result_text \
             FROM audio_segments \
             JOIN (\
                SELECT subject_id, MAX(id) AS id \
                FROM processing_results \
                WHERE subject_type = ?1 AND processor = ?2 \
                GROUP BY subject_id\
             ) latest_transcription ON latest_transcription.subject_id = audio_segments.id \
             JOIN processing_results ON processing_results.id = latest_transcription.id \
             WHERE audio_segments.started_at <= ?4 AND audio_segments.ended_at >= ?3 \
               AND LENGTH(TRIM(COALESCE(processing_results.result_text, ''))) > 0 \
             ORDER BY audio_segments.started_at ASC, audio_segments.id ASC \
             LIMIT ?5",
        )
        .bind(AUDIO_SEGMENT_SUBJECT_TYPE)
        .bind(AUDIO_TRANSCRIPTION_PROCESSOR)
        .bind(&start_rfc3339)
        .bind(&end_rfc3339)
        .bind(max_items)
        .fetch_all(self.pool())
        .await?;

        for row in audio_rows {
            let started_at: String = row.get("started_at");
            let Some(captured_at_ms) = rfc3339_to_ms(&started_at) else {
                continue;
            };
            let text: String = row.get("result_text");
            items.push(CaptureWindowItem {
                subject_type: AUDIO_SEGMENT_SUBJECT_TYPE.to_string(),
                subject_id: row.get("subject_id"),
                captured_at_ms,
                text,
                app_label: None,
                url: None,
            });
        }

        // Merge the two time-ordered streams, then re-cap at max_items.
        items.sort_by(|a, b| {
            a.captured_at_ms
                .cmp(&b.captured_at_ms)
                .then_with(|| a.subject_type.cmp(&b.subject_type))
                .then_with(|| a.subject_id.cmp(&b.subject_id))
        });
        if max_items >= 0 {
            items.truncate(max_items as usize);
        }

        Ok(CaptureWindow {
            items,
            start_ms,
            end_ms,
        })
    }
}

/// Builds a best-effort `(app_label, url)` pair from a frame metadata snapshot
/// JSON blob. Prefers the app name, falling back to the bundle id, and appends
/// the window title when present.
fn search_context_from_snapshot(
    snapshot_json: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(snapshot_json) = snapshot_json else {
        return (None, None);
    };
    let Ok(snapshot) =
        serde_json::from_str::<capture_metadata::FrameMetadataSnapshot>(snapshot_json)
    else {
        return (None, None);
    };

    let app = snapshot.app_name.or(snapshot.app_bundle_id);
    let app_label = match (app, snapshot.window_title) {
        (Some(app), Some(title)) if !title.trim().is_empty() => Some(format!("{app} — {title}")),
        (Some(app), _) => Some(app),
        (None, Some(title)) if !title.trim().is_empty() => Some(title),
        (None, _) => None,
    };

    (app_label, snapshot.browser_url)
}

/// Converts unix milliseconds to an RFC3339 string for comparison against the
/// legacy RFC3339 TEXT columns (`frames.captured_at`, `audio_segments.*`).
pub(crate) fn ms_to_rfc3339(ms: i64) -> String {
    let nanos = (ms as i128) * 1_000_000;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
        .unwrap_or_default()
}

/// Converts an RFC3339 TEXT timestamp to unix milliseconds; `None` on a parse
/// failure (the row is then skipped rather than poisoning the window).
fn rfc3339_to_ms(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .map(|dt| (dt.unix_timestamp_nanos() / 1_000_000) as i64)
}
