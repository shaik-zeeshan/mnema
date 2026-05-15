CREATE TABLE IF NOT EXISTS captured_screen_text (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    frame_id INTEGER NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    source TEXT NOT NULL CHECK (source IN ('accessibility')),
    result_text TEXT NOT NULL CHECK (LENGTH(TRIM(result_text)) > 0),
    structured_payload_json TEXT,
    captured_at_unix_ms INTEGER NOT NULL,
    source_app_bundle_id TEXT,
    source_app_name TEXT,
    source_window_title TEXT,
    source_window_id INTEGER,
    snapshot_age_ms INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(frame_id, source)
);

CREATE INDEX IF NOT EXISTS captured_screen_text_frame_source_idx
    ON captured_screen_text (frame_id, source);
CREATE INDEX IF NOT EXISTS captured_screen_text_source_id_idx
    ON captured_screen_text (source, id DESC);
