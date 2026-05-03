ALTER TABLE frames
    ADD COLUMN equivalence_hint TEXT
    CHECK (equivalence_hint IS NULL OR LENGTH(TRIM(equivalence_hint)) > 0);

ALTER TABLE frames
    ADD COLUMN equivalence_proof BLOB;

ALTER TABLE frames
    ADD COLUMN equivalence_version INTEGER
    CHECK (equivalence_version IS NULL OR equivalence_version > 0);

ALTER TABLE frames
    ADD COLUMN equivalence_status TEXT
    CHECK (equivalence_status IS NULL OR equivalence_status IN ('ready', 'quarantined'));

ALTER TABLE frames
    ADD COLUMN equivalence_error TEXT;

CREATE INDEX IF NOT EXISTS frames_session_equivalence_hint_idx
    ON frames (session_id, equivalence_hint, id ASC);
