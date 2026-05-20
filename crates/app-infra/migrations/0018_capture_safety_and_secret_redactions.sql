CREATE TABLE IF NOT EXISTS capture_safety_gaps (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_session_id TEXT NOT NULL REFERENCES capture_sessions(capture_session_id) ON DELETE CASCADE,
    reason TEXT NOT NULL CHECK (reason IN ('credential_entry')),
    started_at TEXT NOT NULL,
    ended_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS capture_safety_gaps_session_idx
    ON capture_safety_gaps(capture_session_id, started_at, id);

CREATE INDEX IF NOT EXISTS capture_safety_gaps_time_idx
    ON capture_safety_gaps(started_at, ended_at, id);

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
