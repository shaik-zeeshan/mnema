-- Single-row projection of the Licensing state (ADR 0045). The OS keychain is
-- the source of truth for the signed license key + trial record; this table is
-- a fast-read cache for the startup gate and the Settings UI, plus the
-- anti-rollback high-water mark.
CREATE TABLE IF NOT EXISTS licensing_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    trial_started_at_ms INTEGER,        -- NULL until first successful Capture
    max_timestamp_ever_seen_ms INTEGER NOT NULL DEFAULT 0,  -- anti-rollback high-water mark
    license_id TEXT,
    tier TEXT,
    issued_at_ms INTEGER,
    update_through_ms INTEGER,
    email TEXT
);
INSERT OR IGNORE INTO licensing_state (id) VALUES (1);
