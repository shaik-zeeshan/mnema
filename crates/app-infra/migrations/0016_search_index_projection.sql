CREATE TABLE IF NOT EXISTS search_documents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    anchor_type TEXT NOT NULL CHECK (anchor_type IN ('frame', 'audio')),
    frame_id INTEGER REFERENCES frames (id) ON DELETE CASCADE,
    audio_segment_id INTEGER REFERENCES audio_segments (id) ON DELETE CASCADE,
    processing_result_id INTEGER REFERENCES processing_results (id) ON DELETE SET NULL,
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
    -- `equivalent_reuse` rows do NOT copy the canonical frame's OCR text: they
    -- store NULL `body_text` and borrow the canonical `direct` row's text through
    -- `canonical_search_document_id` (visually-identical frames share one copy).
    -- `direct` rows own their text. The CHECK below makes that the only legal shape.
    body_text TEXT,
    canonical_search_document_id INTEGER REFERENCES search_documents (id) ON DELETE CASCADE,
    context_text TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CHECK (
        (anchor_type = 'frame' AND frame_id IS NOT NULL AND audio_segment_id IS NULL)
        OR (anchor_type = 'audio' AND audio_segment_id IS NOT NULL)
    ),
    CHECK (
        (text_source_kind = 'equivalent_reuse'
            AND body_text IS NULL
            AND canonical_search_document_id IS NOT NULL)
        OR (text_source_kind = 'direct'
            AND body_text IS NOT NULL AND LENGTH(TRIM(body_text)) > 0
            AND canonical_search_document_id IS NULL)
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
CREATE INDEX IF NOT EXISTS search_documents_canonical_idx
    ON search_documents (canonical_search_document_id);

CREATE VIRTUAL TABLE IF NOT EXISTS search_documents_fts USING fts5(
    body_text,
    context_text,
    content='search_documents',
    content_rowid='id',
    tokenize='unicode61'
);

-- `equivalent_reuse` rows are never inserted into the FTS index (they carry no
-- body_text of their own), so their delete must NOT issue an FTS `delete` op —
-- doing so would corrupt the external-content index with a row it never held.
CREATE TRIGGER IF NOT EXISTS search_documents_fts_after_delete
AFTER DELETE ON search_documents
WHEN OLD.text_source_kind <> 'equivalent_reuse'
BEGIN
    INSERT INTO search_documents_fts(search_documents_fts, rowid, body_text, context_text)
    VALUES('delete', OLD.id, OLD.body_text, OLD.context_text);
END;

CREATE TRIGGER IF NOT EXISTS search_documents_direct_after_result_delete
AFTER DELETE ON processing_results
BEGIN
    DELETE FROM search_documents
    WHERE text_source_kind = 'direct'
      AND processing_result_id IS NULL;
END;
