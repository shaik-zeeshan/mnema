CREATE TABLE IF NOT EXISTS search_documents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    anchor_type TEXT NOT NULL CHECK (anchor_type IN ('frame', 'audio')),
    frame_id INTEGER REFERENCES frames (id) ON DELETE CASCADE,
    audio_segment_id INTEGER REFERENCES audio_segments (id) ON DELETE CASCADE,
    processing_result_id INTEGER NOT NULL REFERENCES processing_results (id) ON DELETE CASCADE,
    span_start_ms INTEGER,
    span_end_ms INTEGER,
    absolute_start_at TEXT NOT NULL CHECK (LENGTH(TRIM(absolute_start_at)) > 0),
    absolute_end_at TEXT NOT NULL CHECK (LENGTH(TRIM(absolute_end_at)) > 0),
    source_kind TEXT CHECK (source_kind IS NULL OR source_kind IN ('microphone', 'system_audio')),
    session_id TEXT NOT NULL CHECK (LENGTH(TRIM(session_id)) > 0),
    app_name TEXT,
    window_title TEXT,
    group_key TEXT NOT NULL CHECK (LENGTH(TRIM(group_key)) > 0),
    text_source_kind TEXT NOT NULL CHECK (text_source_kind IN ('direct', 'equivalent_reuse')),
    searchable_text TEXT NOT NULL CHECK (LENGTH(TRIM(searchable_text)) > 0),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (
        (anchor_type = 'frame' AND frame_id IS NOT NULL AND audio_segment_id IS NULL)
        OR (anchor_type = 'audio' AND audio_segment_id IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS search_documents_anchor_idx
    ON search_documents (anchor_type, group_key, absolute_start_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS search_documents_frame_idx
    ON search_documents (frame_id);
CREATE INDEX IF NOT EXISTS search_documents_audio_idx
    ON search_documents (audio_segment_id);
CREATE INDEX IF NOT EXISTS search_documents_result_idx
    ON search_documents (processing_result_id);

CREATE VIRTUAL TABLE IF NOT EXISTS search_documents_fts USING fts5(
    searchable_text,
    content='search_documents',
    content_rowid='id',
    tokenize='unicode61'
);
