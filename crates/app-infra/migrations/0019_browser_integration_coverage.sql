ALTER TABLE capture_safety_gaps
    ADD COLUMN source_family TEXT NOT NULL DEFAULT 'native_secure_entry'
    CHECK (source_family IN ('native_secure_entry', 'browser_secure_entry', 'mixed'));

ALTER TABLE capture_safety_gaps
    ADD COLUMN terminal_status TEXT NOT NULL DEFAULT 'cleared'
    CHECK (terminal_status IN ('cleared', 'source_lost_fail_closed', 'recording_stopped', 'user_pause_took_over'));

CREATE TABLE IF NOT EXISTS browser_integration_coverage_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at TEXT NOT NULL,
    browser_family TEXT NOT NULL CHECK (browser_family IN ('safari', 'chromium')),
    state TEXT NOT NULL CHECK (state IN ('active', 'clear', 'unavailable', 'available')),
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS browser_integration_coverage_events_time_idx
    ON browser_integration_coverage_events(occurred_at, id);
