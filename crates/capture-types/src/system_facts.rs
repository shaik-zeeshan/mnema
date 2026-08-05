use serde::{Deserialize, Serialize};

/// The machine facts Settings needs to write an honest consequence next to a
/// control ("at this rate, ~X GB/day"). Round-4 decision **G8**: a denominator
/// ships ONLY where the value is real on this machine, so every field here is
/// `Option` — an unmeasurable fact is `None` and the UI renders no number at
/// all rather than a placeholder.
///
/// Deliberately absent, per G8: any temperature, and any minute-precise ETA.
/// Nothing here can be turned into either.
///
/// Model download sizes are NOT mirrored here. They already reach the frontend
/// on the per-subsystem model-status DTOs (`byteSize` / `approxDownloadBytes`),
/// sourced from the crate manifests, which are the corrected registry G8 asks
/// for (speakrs `419_482_724`, asserted against its 76-file table by a test in
/// `crates/speaker-analysis`). A second copy here would be the drift G8 warns
/// about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemFacts {
    /// The capture root that every disk figure below was measured against.
    pub capture_path: String,
    /// Free bytes on the capture volume. `None` when the volume can't be read.
    pub disk_free_bytes: Option<u64>,
    /// Physical RAM. `None` off macOS or when the sysctl fails.
    pub total_ram_bytes: Option<u64>,
    /// Bytes/day MEASURED from the recordings tree, averaged over
    /// `measured_days` complete day-directories. `None` until at least one
    /// complete day of capture exists — there is nothing honest to say before
    /// then.
    pub measured_bytes_per_day: Option<u64>,
    /// How many day-directories the average covers (0 when it is `None`).
    pub measured_days: u32,
    /// The screen capture rate currently configured, so the frontend can
    /// project the measured rate onto a different slider position (storage
    /// scales linearly with frame rate at a preset bitrate).
    pub screen_frame_rate: Option<f64>,
    /// Screen frames waiting for OCR (queued + running).
    pub ocr_backlog: Option<i64>,
    /// Audio segments waiting for transcription (queued + running).
    pub transcription_backlog: Option<i64>,
    /// Semantic Search Vectors already stored.
    pub semantic_vector_count: Option<i64>,
    /// Search anchors that have no vector yet — what enabling semantic search
    /// would still have to index.
    pub semantic_pending_count: Option<i64>,
    /// Bytes of one Semantic Search Vector at rest: `int8[768]` per migration
    /// `0039`, so the index cost of N anchors is N × this. A schema fact, not
    /// an estimate.
    pub semantic_vector_bytes: u64,
    /// The capture index (SQLite) file size.
    pub database_bytes: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_facts_round_trips_camel_case() {
        let facts = SystemFacts {
            capture_path: "/Users/x/.mnema".to_string(),
            disk_free_bytes: Some(123_456_789),
            total_ram_bytes: Some(17_179_869_184),
            measured_bytes_per_day: Some(4_000_000_000),
            measured_days: 5,
            screen_frame_rate: Some(0.5),
            ocr_backlog: Some(42),
            transcription_backlog: None,
            semantic_vector_count: Some(0),
            semantic_pending_count: Some(9_000),
            semantic_vector_bytes: 768,
            database_bytes: Some(2_048),
        };

        let json = serde_json::to_string(&facts).expect("serialize");
        assert!(json.contains("\"measuredBytesPerDay\":4000000000"));
        assert!(json.contains("\"screenFrameRate\":0.5"));
        assert!(json.contains("\"transcriptionBacklog\":null"));
        assert!(json.contains("\"semanticVectorBytes\":768"));

        let back: SystemFacts = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, facts);
    }
}
