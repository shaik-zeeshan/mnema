-- User Context (issue #88) storage foundation: Activities (the evidence layer)
-- plus their per-capture evidence rows and the derivation-run ledger.
--
-- Timestamp convention for every `user_context_*` table: INTEGER unix
-- milliseconds columns named `*_at_ms`, set from Rust at insert (NOT
-- CURRENT_TIMESTAMP). This avoids RFC3339 string parsing inside user_context;
-- the capture-window reader converts the legacy RFC3339 `frames.captured_at` /
-- `audio_segments.started_at` to millis at the boundary.

-- Derivation runs: which windows are covered (newest-first, no re-derive) + the
-- token-usage readout. Created BEFORE user_context_activities because the
-- activities table carries a forward FK reference to this table (a forward FK
-- reference is fine in SQLite when the referenced table is created earlier in
-- the same migration).
CREATE TABLE IF NOT EXISTS user_context_derivation_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,                   -- 'activity' | 'conclusion' | 'confidence' | 'backfill'
    window_start_ms INTEGER,
    window_end_ms INTEGER,
    status TEXT NOT NULL DEFAULT 'completed',  -- 'running' | 'completed' | 'failed' | 'skipped'
    activities_derived INTEGER NOT NULL DEFAULT 0,
    conclusions_derived INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    provider TEXT,
    model TEXT,
    error TEXT,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS user_context_derivation_runs_window_idx
    ON user_context_derivation_runs (window_start_ms, window_end_ms);
CREATE INDEX IF NOT EXISTS user_context_derivation_runs_kind_idx
    ON user_context_derivation_runs (kind, created_at_ms);

CREATE TABLE IF NOT EXISTS user_context_activities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    category TEXT,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER NOT NULL,
    derivation_run_id INTEGER REFERENCES user_context_derivation_runs(id) ON DELETE SET NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS user_context_activities_time_idx
    ON user_context_activities (started_at_ms, ended_at_ms, id);

-- Evidence: Activity -> raw captures. NO FK to frames/audio_segments because
-- derived data must SURVIVE Retention Policy aging (ADR 0029). Delete Recent
-- Capture purges rows here explicitly via the cascade.
CREATE TABLE IF NOT EXISTS user_context_activity_evidence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    activity_id INTEGER NOT NULL REFERENCES user_context_activities(id) ON DELETE CASCADE,
    subject_type TEXT NOT NULL,           -- 'frame' | 'audio_segment'
    subject_id INTEGER NOT NULL,
    captured_at_ms INTEGER,
    UNIQUE (activity_id, subject_type, subject_id)
);
CREATE INDEX IF NOT EXISTS user_context_activity_evidence_activity_idx
    ON user_context_activity_evidence (activity_id);
CREATE INDEX IF NOT EXISTS user_context_activity_evidence_subject_idx
    ON user_context_activity_evidence (subject_type, subject_id);
