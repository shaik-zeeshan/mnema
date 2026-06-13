//! Capture-window reader for the User Context derivation worker.
//!
//! Assembles the already-redacted OCR / transcript **text** (plus best-effort
//! Search Context app/url labels) for captures inside a time window, so the
//! derivation worker can hand a window to the Reasoning Engine.
//!
//! Privacy invariant (spec §7): this reader only ever reads
//! `processing_results.result_text` — the already-redacted OCR / transcript
//! text — plus a Search Context label drawn from the frame metadata snapshot. It
//! never reads frame images or audio files.
//!
//! Metadata egress caveat (PR #112 #7): the secret-redaction pipeline rewrites
//! only `result_text`, NOT the metadata snapshot. So `app_label`/`url` are NOT
//! redacted the way `result_text` is. A `BrowserUrlMode::Full` URL can carry
//! tokens/PII in its path/query, and a window title can carry document/recipient
//! names. Because this window is handed to a (possibly cloud) Reasoning Engine, we
//! reduce the metadata to a privacy-safe form at read time:
//! [`search_context_from_snapshot`] drops the window title (keeping only the app
//! name) and reduces any browser URL to its bare origin host (scheme/path/query/
//! fragment stripped). The redacted `result_text` remains the only free text sent.

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

/// Builds a privacy-reduced best-effort `(app_label, url)` pair from a frame
/// metadata snapshot JSON blob, for egress to a (possibly cloud) Reasoning Engine.
///
/// The snapshot metadata is NOT covered by the secret-redaction pipeline (which
/// only rewrites `result_text`), so this deliberately keeps only the low-risk
/// signal (PR #112 #7):
/// - `app_label` is the **app name only** (falling back to the bundle id). The
///   **window title is dropped** — titles carry document/chat/recipient names.
/// - `url` is reduced to its **bare origin host** via [`url_origin_host`]
///   (scheme, userinfo, path, query, fragment, and port all stripped) — a
///   `BrowserUrlMode::Full` URL can otherwise carry tokens/PII in its path/query.
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

    // App name only (bundle id fallback). Window title is intentionally dropped.
    let app_label = snapshot.app_name.or(snapshot.app_bundle_id);
    // Reduce any browser URL to its bare origin host before it can egress.
    let url = snapshot
        .browser_url
        .as_deref()
        .and_then(url_origin_host);

    (app_label, url)
}

/// Reduces a URL string to its bare lowercase origin host: strips the scheme, any
/// userinfo, the path/query/fragment, and the port. Returns `None` for an
/// empty/host-less value. Avoids pulling in the `url` crate; mirrors
/// `usage_charts::domain_from_url`. This is the privacy reduction applied to a
/// browser URL before it is handed to the Reasoning Engine (PR #112 #7).
fn url_origin_host(raw: &str) -> Option<String> {
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
    // Drop port. IPv6 bracket forms keep the bracketed host.
    let host = if host_port.starts_with('[') {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_origin_host_reduces_to_bare_host() {
        // Path, query, and fragment are all dropped — these carry tokens/PII.
        assert_eq!(
            url_origin_host("https://mail.example.com/u/0/inbox?token=secret#x"),
            Some("mail.example.com".to_string())
        );
        // Userinfo and port dropped, host lowercased.
        assert_eq!(
            url_origin_host("http://user:pass@Example.COM:8443/path"),
            Some("example.com".to_string())
        );
        // Scheme-less input still yields the host.
        assert_eq!(url_origin_host("docs.rs/foo"), Some("docs.rs".to_string()));
        // Empty / host-less inputs.
        assert_eq!(url_origin_host(""), None);
        assert_eq!(url_origin_host("https://"), None);
    }

    #[test]
    fn snapshot_drops_window_title_and_reduces_url() {
        let snapshot = capture_metadata::FrameMetadataSnapshot {
            app_name: Some("Safari".to_string()),
            app_bundle_id: Some("com.apple.Safari".to_string()),
            // A title carrying a recipient/document name — must NOT egress.
            window_title: Some("Re: contract for Jane Doe — Mail".to_string()),
            // A Full-mode URL carrying a token in its query — must be reduced.
            browser_url: Some("https://mail.example.com/inbox?auth=topsecret".to_string()),
            ..Default::default()
        };
        let json = snapshot.normalized_json();
        let (app_label, url) = search_context_from_snapshot(Some(&json));

        // App name only — the window title is dropped entirely.
        assert_eq!(app_label, Some("Safari".to_string()));
        let label = app_label.unwrap();
        assert!(!label.contains("Jane Doe"), "window title leaked: {label}");
        assert!(!label.contains("contract"), "window title leaked: {label}");

        // URL reduced to bare origin host — no path, no token.
        assert_eq!(url, Some("mail.example.com".to_string()));
        let url = url.unwrap();
        assert!(!url.contains("topsecret"), "url query leaked: {url}");
        assert!(!url.contains("inbox"), "url path leaked: {url}");
    }

    #[test]
    fn snapshot_falls_back_to_bundle_id_when_no_app_name() {
        let snapshot = capture_metadata::FrameMetadataSnapshot {
            app_name: None,
            app_bundle_id: Some("com.acme.tool".to_string()),
            window_title: Some("Secret Document".to_string()),
            browser_url: None,
            ..Default::default()
        };
        let json = snapshot.normalized_json();
        let (app_label, url) = search_context_from_snapshot(Some(&json));
        assert_eq!(app_label, Some("com.acme.tool".to_string()));
        assert_eq!(url, None);
    }

    #[test]
    fn snapshot_none_or_unparseable_yields_no_context() {
        assert_eq!(search_context_from_snapshot(None), (None, None));
        assert_eq!(search_context_from_snapshot(Some("not json")), (None, None));
    }
}
