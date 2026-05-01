CREATE TABLE IF NOT EXISTS audio_segments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('microphone', 'system_audio')),
    source_session_id TEXT NOT NULL CHECK (LENGTH(TRIM(source_session_id)) > 0),
    segment_index INTEGER NOT NULL CHECK (segment_index > 0),
    file_path TEXT NOT NULL CHECK (LENGTH(TRIM(file_path)) > 0),
    started_at TEXT NOT NULL CHECK (LENGTH(TRIM(started_at)) > 0),
    ended_at TEXT NOT NULL CHECK (LENGTH(TRIM(ended_at)) > 0),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (source_kind, source_session_id, file_path),
    UNIQUE (source_kind, source_session_id, segment_index, file_path)
);

CREATE INDEX IF NOT EXISTS audio_segments_time_range_idx
    ON audio_segments (started_at, ended_at, id);
CREATE INDEX IF NOT EXISTS audio_segments_source_session_idx
    ON audio_segments (source_kind, source_session_id, segment_index, id);
