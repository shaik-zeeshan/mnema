CREATE TABLE IF NOT EXISTS secret_redactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    anchor_type TEXT NOT NULL CHECK (anchor_type IN ('frame', 'audio')),
    frame_id INTEGER REFERENCES frames(id) ON DELETE CASCADE,
    audio_segment_id INTEGER REFERENCES audio_segments(id) ON DELETE CASCADE,
    processing_result_id INTEGER REFERENCES processing_results(id) ON DELETE CASCADE,
    category TEXT NOT NULL,
    redacted_start INTEGER NOT NULL,
    redacted_end INTEGER NOT NULL,
    detector_version TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (
        (anchor_type = 'frame' AND frame_id IS NOT NULL AND audio_segment_id IS NULL)
        OR (anchor_type = 'audio' AND audio_segment_id IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS secret_redactions_anchor_idx
    ON secret_redactions(anchor_type, frame_id, audio_segment_id, id);

CREATE INDEX IF NOT EXISTS secret_redactions_result_idx
    ON secret_redactions(processing_result_id, id);
