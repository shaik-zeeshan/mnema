CREATE TABLE IF NOT EXISTS frame_metadata_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    normalized_hash TEXT NOT NULL CHECK (LENGTH(TRIM(normalized_hash)) > 0),
    snapshot_json TEXT NOT NULL CHECK (LENGTH(TRIM(snapshot_json)) > 0),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (normalized_hash)
);

CREATE INDEX IF NOT EXISTS frame_metadata_snapshots_hash_idx
    ON frame_metadata_snapshots(normalized_hash);

ALTER TABLE frames ADD COLUMN metadata_snapshot_id INTEGER REFERENCES frame_metadata_snapshots(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS frames_metadata_snapshot_id_idx
    ON frames(metadata_snapshot_id);
