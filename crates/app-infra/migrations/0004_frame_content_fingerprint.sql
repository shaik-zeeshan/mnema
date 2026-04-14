ALTER TABLE frames
    ADD COLUMN content_fingerprint TEXT
    CHECK (content_fingerprint IS NULL OR LENGTH(TRIM(content_fingerprint)) > 0);

CREATE INDEX IF NOT EXISTS frames_session_fingerprint_idx
    ON frames (session_id, content_fingerprint, id DESC);
