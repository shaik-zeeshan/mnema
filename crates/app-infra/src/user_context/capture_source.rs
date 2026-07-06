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
//! [`search_context_from_parsed`] drops the window title (keeping only the app
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
    /// Audio only — the capture source: `"microphone"` (the user's own voice) or
    /// `"system_audio"` (the other party). `None` for frames. Lets the derivation
    /// prompt tag speech `you` vs `other-side` so words spoken TO the user are not
    /// misattributed as words the user said (ADR 0050).
    pub source_kind: Option<String>,
    /// Audio only — diarized speaker turns as `(cluster_id, transcript_text)`,
    /// time-ordered. ANONYMOUS BY CONSTRUCTION (ADR 0050): the reader never selects
    /// a `person_id` / `display_name` / any name column into this, so a recognized
    /// person's name is UNREPRESENTABLE here and can never reach the (possibly
    /// cloud) Reasoning Engine — names resolve on-device only. Empty for frames and
    /// for audio whose diarization has produced no turns (yet).
    pub speaker_turns: Vec<(i64, String)>,
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
            let snapshot = parse_snapshot(snapshot_json.as_deref());
            // Skip frames of Mnema's own UI (best-effort: frames without a
            // snapshot cannot be identified and pass through). ponytail: self
            // frames still eat the SQL LIMIT budget; push the filter into SQL
            // json_extract if that ever matters.
            if snapshot.as_ref().is_some_and(is_self_capture) {
                continue;
            }
            let (app_label, url) = snapshot.map_or((None, None), search_context_from_parsed);

            items.push(CaptureWindowItem {
                subject_type: FRAME_SUBJECT_TYPE.to_string(),
                subject_id: row.get("subject_id"),
                captured_at_ms,
                text,
                app_label,
                url,
                source_kind: None,
                speaker_turns: Vec::new(),
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
                    audio_segments.source_kind AS source_kind, \
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
            let subject_id: i64 = row.get("subject_id");
            let text: String = row.get("result_text");
            let source_kind: Option<String> = row.get("source_kind");

            // Anonymous diarized turns for this segment (ADR 0050): read ONLY
            // (cluster_id, transcript_text) — deliberately NOT `person_id` /
            // `display_name` / any name column — so a recognized person's name is
            // UNREPRESENTABLE in the window handed to the (possibly cloud) engine.
            // Names resolve on-device only, never over the wire.
            // ponytail: one query per audio segment (N+1), bounded by max_items;
            // batch by `audio_segment_id IN (...)` only if a wide window makes it matter.
            let turn_rows = sqlx::query(
                "SELECT cluster_id, transcript_text \
                 FROM speaker_turns \
                 WHERE audio_segment_id = ?1 \
                   AND LENGTH(TRIM(COALESCE(transcript_text, ''))) > 0 \
                 ORDER BY start_ms ASC, end_ms ASC, id ASC",
            )
            .bind(subject_id)
            .fetch_all(self.pool())
            .await?;
            let speaker_turns: Vec<(i64, String)> = turn_rows
                .iter()
                .map(|turn| (turn.get::<i64, _>("cluster_id"), turn.get::<String, _>("transcript_text")))
                .collect();

            items.push(CaptureWindowItem {
                subject_type: AUDIO_SEGMENT_SUBJECT_TYPE.to_string(),
                subject_id,
                captured_at_ms,
                text,
                app_label: None,
                url: None,
                source_kind,
                speaker_turns,
            });
        }

        // Merge the two time-ordered streams, collapse runs of near-identical
        // frames (a screen held static at 1fps otherwise floods the prompt with
        // dozens of duplicate OCR blocks), then re-cap at max_items. Dedup runs
        // BEFORE the truncate so the cap keeps compacted (distinct) content.
        items.sort_by(|a, b| {
            a.captured_at_ms
                .cmp(&b.captured_at_ms)
                .then_with(|| a.subject_type.cmp(&b.subject_type))
                .then_with(|| a.subject_id.cmp(&b.subject_id))
        });
        let mut items = dedup_adjacent_frames(items);
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

/// Word-set Jaccard at or above which two consecutive same-app frames are treated
/// as the same screen and collapsed. 0.9 tolerates OCR jitter (a blinking cursor,
/// a ticking clock, a changing % readout) without merging genuinely different
/// screens; a starting point to tune against real windows like the ADR 0042 floor.
const FRAME_DEDUP_JACCARD_THRESHOLD: f64 = 0.9;

/// Lowercased whitespace-token set of `text`, for near-duplicate frame detection.
/// Punctuation stays attached to tokens — OCR noise is the target, and exact-token
/// overlap is a good-enough signal without pulling in a tokenizer. `// ponytail:
/// word-set Jaccard, upgrade to shingles/edit-distance only if jitter still leaks.`
fn word_set(text: &str) -> std::collections::HashSet<String> {
    text.split_whitespace().map(str::to_lowercase).collect()
}

/// Jaccard similarity of two frames' word sets: `|A∩B| / |A∪B|`. Two empty texts
/// count as identical (1.0); one empty as disjoint (0.0).
fn text_similarity(a: &str, b: &str) -> f64 {
    let (sa, sb) = (word_set(a), word_set(b));
    let union = sa.union(&sb).count();
    if union == 0 {
        return 1.0;
    }
    sa.intersection(&sb).count() as f64 / union as f64
}

/// Collapse runs of CONSECUTIVE near-identical frames from the SAME app into one
/// representative (the frame with the most OCR text), so a static screen captured
/// at 1fps does not flood the derivation prompt with duplicate blocks. Only frames
/// collapse, and only against the immediately-preceding KEPT frame of the same
/// `app_label`: audio transcript items and app switches break a run, so a genuine
/// return to a screen after other work stays a distinct item. Dropped duplicates
/// lose their own evidence tag — the representative frame carries the run's
/// evidence, which is the intent (one clean frame, not sixty identical ones).
fn dedup_adjacent_frames(items: Vec<CaptureWindowItem>) -> Vec<CaptureWindowItem> {
    let mut out: Vec<CaptureWindowItem> = Vec::with_capacity(items.len());
    for item in items {
        if item.subject_type == FRAME_SUBJECT_TYPE {
            if let Some(last) = out.last_mut() {
                if last.subject_type == FRAME_SUBJECT_TYPE
                    && last.app_label == item.app_label
                    && text_similarity(&last.text, &item.text) >= FRAME_DEDUP_JACCARD_THRESHOLD
                {
                    // Near-duplicate of the current representative: keep whichever
                    // carries the richer OCR text, drop the other.
                    if item.text.chars().count() > last.text.chars().count() {
                        *last = item;
                    }
                    continue;
                }
            }
        }
        out.push(item);
    }
    out
}

/// Parses a frame metadata snapshot JSON blob; `None` when absent or unparseable.
fn parse_snapshot(
    snapshot_json: Option<&str>,
) -> Option<capture_metadata::FrameMetadataSnapshot> {
    serde_json::from_str(snapshot_json?).ok()
}

/// Mnema's own app identities (prod + dev builds). Frames of Mnema's own UI are
/// excluded from the capture window: otherwise the app OCRs its own generated
/// insights/digests and re-ingests them as activity evidence (a self-capture
/// feedback loop).
const SELF_APP_BUNDLE_IDS: &[&str] = &["com.shaikzeeshan.mnema", "com.shaikzeeshan.mnema.dev"];
const SELF_APP_NAMES: &[&str] = &["mnema", "mnema-dev"];

/// True when a frame's metadata snapshot identifies Mnema itself: exact bundle-id
/// match OR case-insensitive app-name match.
fn is_self_capture(snapshot: &capture_metadata::FrameMetadataSnapshot) -> bool {
    snapshot
        .app_bundle_id
        .as_deref()
        .is_some_and(|id| SELF_APP_BUNDLE_IDS.contains(&id))
        || snapshot.app_name.as_deref().is_some_and(|name| {
            SELF_APP_NAMES
                .iter()
                .any(|own| name.eq_ignore_ascii_case(own))
        })
}

/// Builds a privacy-reduced best-effort `(app_label, url)` pair from a parsed
/// frame metadata snapshot, for egress to a (possibly cloud) Reasoning Engine.
///
/// The snapshot metadata is NOT covered by the secret-redaction pipeline (which
/// only rewrites `result_text`), so this deliberately keeps only the low-risk
/// signal (PR #112 #7):
/// - `app_label` is the **app name only** (falling back to the bundle id). The
///   **window title is dropped** — titles carry document/chat/recipient names.
/// - `url` is reduced to its **bare origin host** via [`url_origin_host`]
///   (scheme, userinfo, path, query, fragment, and port all stripped) — a
///   `BrowserUrlMode::Full` URL can otherwise carry tokens/PII in its path/query.
fn search_context_from_parsed(
    snapshot: capture_metadata::FrameMetadataSnapshot,
) -> (Option<String>, Option<String>) {
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

    /// Test convenience: parse-then-reduce in one step, as the frame loop does.
    fn search_context_from_snapshot(
        snapshot_json: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        parse_snapshot(snapshot_json).map_or((None, None), search_context_from_parsed)
    }

    fn frame(id: i64, t: i64, text: &str, app: &str) -> CaptureWindowItem {
        CaptureWindowItem {
            subject_type: FRAME_SUBJECT_TYPE.to_string(),
            subject_id: id,
            captured_at_ms: t,
            text: text.to_string(),
            app_label: Some(app.to_string()),
            url: None,
            source_kind: None,
            speaker_turns: Vec::new(),
        }
    }

    fn audio(id: i64, t: i64, text: &str, source: &str) -> CaptureWindowItem {
        CaptureWindowItem {
            subject_type: AUDIO_SEGMENT_SUBJECT_TYPE.to_string(),
            subject_id: id,
            captured_at_ms: t,
            text: text.to_string(),
            app_label: None,
            url: None,
            source_kind: Some(source.to_string()),
            speaker_turns: Vec::new(),
        }
    }

    #[test]
    fn dedup_adjacent_frames_collapses_static_screen_runs() {
        // A static screen: 20 shared tokens plus one jittering readout token.
        let s = |extra: &str| -> String {
            let mut v: Vec<String> = (0..20).map(|i| format!("w{i}")).collect();
            v.extend(extra.split_whitespace().map(str::to_string));
            v.join(" ")
        };
        // Jitter-only difference is treated as the same screen (>= 0.9)...
        assert!(text_similarity(&s("12%"), &s("13%")) >= FRAME_DEDUP_JACCARD_THRESHOLD);
        // ...but genuinely different content is not.
        assert!(
            text_similarity(&s("12%"), "totally unrelated different screen text here now")
                < FRAME_DEDUP_JACCARD_THRESHOLD
        );

        let items = vec![
            frame(1, 100, &s("12%"), "Hitch"),       // run representative
            frame(2, 101, &s("12%"), "Hitch"),       // identical dup -> dropped
            frame(3, 102, &s("12% bonus"), "Hitch"), // near-dup + richer -> new representative
            audio(4, 103, "spoken words here", "microphone"), // audio breaks the run
            frame(5, 104, &s("12%"), "Hitch"),       // new run after audio -> kept
            frame(6, 105, &s("12%"), "Zen"),         // app switch -> kept
        ];
        let out = dedup_adjacent_frames(items);

        // 1/2/3 collapse to the richest (id 3); audio, post-audio frame, and the
        // app-switched frame all survive.
        let ids: Vec<i64> = out.iter().map(|i| i.subject_id).collect();
        assert_eq!(ids, vec![3, 4, 5, 6]);
        // The representative carries the richest OCR text of its run.
        assert_eq!(out[0].text, s("12% bonus"));
    }

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

    /// NAMES-NEVER-TO-CLOUD, exercised against the REAL schema (ADR 0050).
    ///
    /// Unlike the pure-function proxy tests, this seeds a state where the spoken
    /// cluster IS recognized as a named person on-device: `person_profiles` holds
    /// "Jane Doe" and the `recording_speaker_clusters` row the turn points at has
    /// `recognition_person_id` set to her. If `read_capture_window`'s projection
    /// ever joined the cluster to its person (or selected a name column), the name
    /// would surface in the returned window that is handed to the (possibly cloud)
    /// engine. The projection reads ONLY `(cluster_id, transcript_text)`, so the
    /// name must be absent from every field of the returned `CaptureWindow`.
    #[test]
    fn read_capture_window_never_surfaces_a_recognized_person_name() {
        use sqlx::sqlite::SqlitePoolOptions;
        const SENTINEL: &str = "Jane Doe";

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .expect("open db");
            // Minimal real-schema slice the reader touches (avoids the full
            // migrator, which needs the vec0 extension). Mirrors migrations
            // 0003/0007/0010 for the columns under test.
            for statement in [
                "CREATE TABLE frames (id INTEGER PRIMARY KEY, captured_at TEXT NOT NULL, metadata_snapshot_id INTEGER)",
                "CREATE TABLE frame_metadata_snapshots (id INTEGER PRIMARY KEY, snapshot_json TEXT)",
                "CREATE TABLE processing_results (id INTEGER PRIMARY KEY AUTOINCREMENT, job_id INTEGER, subject_type TEXT NOT NULL, subject_id INTEGER NOT NULL, processor TEXT NOT NULL, result_text TEXT, structured_payload_json BLOB)",
                "CREATE TABLE audio_segments (id INTEGER PRIMARY KEY, source_kind TEXT NOT NULL, started_at TEXT NOT NULL, ended_at TEXT NOT NULL)",
                "CREATE TABLE person_profiles (id INTEGER PRIMARY KEY, display_name TEXT NOT NULL)",
                "CREATE TABLE recording_speaker_clusters (id INTEGER PRIMARY KEY, session_id TEXT, provider TEXT, provider_cluster_id TEXT, stable_label TEXT, recognition_person_id INTEGER)",
                "CREATE TABLE speaker_turns (id INTEGER PRIMARY KEY AUTOINCREMENT, audio_segment_id INTEGER NOT NULL, session_id TEXT, cluster_id INTEGER NOT NULL, start_ms INTEGER NOT NULL, end_ms INTEGER NOT NULL, transcript_text TEXT)",
            ] {
                sqlx::query(statement).execute(&pool).await.expect("create table");
            }

            // On-device identity: this cluster resolves to the named person.
            sqlx::query("INSERT INTO person_profiles (id, display_name) VALUES (1, ?1)")
                .bind(SENTINEL)
                .execute(&pool)
                .await
                .expect("seed person");
            sqlx::query(
                "INSERT INTO recording_speaker_clusters \
                    (id, session_id, provider, provider_cluster_id, stable_label, recognition_person_id) \
                 VALUES (1, 's1', 'speakrs', '0', 'Speaker 1', 1)",
            )
            .execute(&pool)
            .await
            .expect("seed cluster");

            sqlx::query(
                "INSERT INTO audio_segments (id, source_kind, started_at, ended_at) \
                 VALUES (1, 'system_audio', '2026-01-01T00:00:00Z', '2026-01-01T00:01:00Z')",
            )
            .execute(&pool)
            .await
            .expect("seed segment");
            sqlx::query(
                "INSERT INTO processing_results (job_id, subject_type, subject_id, processor, result_text) \
                 VALUES (1, ?1, 1, ?2, 'thanks for the update')",
            )
            .bind(AUDIO_SEGMENT_SUBJECT_TYPE)
            .bind(AUDIO_TRANSCRIPTION_PROCESSOR)
            .execute(&pool)
            .await
            .expect("seed transcription");
            sqlx::query(
                "INSERT INTO speaker_turns \
                    (audio_segment_id, session_id, cluster_id, start_ms, end_ms, transcript_text) \
                 VALUES (1, 's1', 1, 0, 1000, 'thanks for the update')",
            )
            .execute(&pool)
            .await
            .expect("seed turn");

            let store = UserContextStore::new(crate::db::CaptureDb::single(pool));
            let window = store
                .read_capture_window(0, 32_503_680_000_000, 100)
                .await
                .expect("read window");

            // The audio item and its anonymous turn are present...
            let audio = window
                .items
                .iter()
                .find(|i| i.subject_type == AUDIO_SEGMENT_SUBJECT_TYPE)
                .expect("audio item present");
            assert_eq!(
                audio.speaker_turns,
                vec![(1_i64, "thanks for the update".to_string())]
            );
            // ...but the recognized person's name never appears in ANY field of
            // the window handed to the engine.
            let dumped = format!("{window:?}");
            assert!(
                !dumped.contains(SENTINEL),
                "recognized person name leaked into the capture window: {dumped}"
            );
        });
    }

    #[test]
    fn self_capture_matches_mnema_identities_only() {
        let snap = |name: Option<&str>, bundle: Option<&str>| {
            capture_metadata::FrameMetadataSnapshot {
                app_name: name.map(str::to_string),
                app_bundle_id: bundle.map(str::to_string),
                ..Default::default()
            }
        };
        // App-name match is case-insensitive, prod and dev.
        assert!(is_self_capture(&snap(Some("mnema"), None)));
        assert!(is_self_capture(&snap(Some("Mnema"), None)));
        assert!(is_self_capture(&snap(Some("MNEMA-DEV"), None)));
        // Bundle-id-only variants (no app name).
        assert!(is_self_capture(&snap(None, Some("com.shaikzeeshan.mnema"))));
        assert!(is_self_capture(&snap(
            None,
            Some("com.shaikzeeshan.mnema.dev")
        )));
        // Other apps pass through, including near-miss names/bundle ids.
        assert!(!is_self_capture(&snap(
            Some("Safari"),
            Some("com.apple.Safari")
        )));
        assert!(!is_self_capture(&snap(Some("mnemanote"), None)));
        assert!(!is_self_capture(&snap(None, Some("com.other.mnema"))));
        assert!(!is_self_capture(&snap(None, None)));
        // The parsed round-trip used by the frame loop detects self frames too.
        let json = snap(Some("mnema-dev"), Some("com.shaikzeeshan.mnema.dev")).normalized_json();
        assert!(parse_snapshot(Some(&json)).is_some_and(|s| is_self_capture(&s)));
    }
}
