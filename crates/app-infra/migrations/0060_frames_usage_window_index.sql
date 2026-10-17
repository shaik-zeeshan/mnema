-- Usage charts read every frame in a calendar window for three columns:
-- `session_id`, `captured_at` and `metadata_snapshot_id`. Through
-- `frames_captured_at_idx` that meant fetching each frame's whole table row
-- (file path, fingerprint, equivalence proof, ...). Insights reruns the read
-- every 60 s while recording, for the current and the previous period, and each
-- capture commit resets the reader's page cache, so every refresh re-read those
-- rows from the file: ~30 MB per call, ~7.8 GB/h of disk reads with Insights
-- open (stmt-cost trace, 2026-10-17). This index covers the read, so it walks
-- only small index entries.
CREATE INDEX IF NOT EXISTS frames_usage_window_idx
    ON frames (captured_at, session_id, metadata_snapshot_id);
