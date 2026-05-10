CREATE TABLE IF NOT EXISTS capture_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_session_id TEXT UNIQUE NOT NULL CHECK (LENGTH(TRIM(capture_session_id)) > 0),
    started_at TEXT NOT NULL CHECK (LENGTH(TRIM(started_at)) > 0),
    stopped_at TEXT,
    status TEXT NOT NULL CHECK (status IN ('recording', 'completed', 'failed')),
    requested_screen INTEGER NOT NULL CHECK (requested_screen IN (0, 1)),
    requested_microphone INTEGER NOT NULL CHECK (requested_microphone IN (0, 1)),
    requested_system_audio INTEGER NOT NULL CHECK (requested_system_audio IN (0, 1)),
    screen_source_session_id TEXT,
    microphone_source_session_id TEXT,
    system_audio_source_session_id TEXT,
    segment_duration_seconds INTEGER NOT NULL CHECK (segment_duration_seconds > 0),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS capture_sessions_started_at_idx
    ON capture_sessions(started_at, id);
CREATE INDEX IF NOT EXISTS capture_sessions_status_idx
    ON capture_sessions(status, id);

CREATE TABLE IF NOT EXISTS capture_segments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_session_id TEXT NOT NULL REFERENCES capture_sessions(capture_session_id) ON DELETE CASCADE,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('screen', 'microphone', 'system_audio')),
    source_session_id TEXT NOT NULL CHECK (LENGTH(TRIM(source_session_id)) > 0),
    segment_index INTEGER NOT NULL CHECK (segment_index > 0),
    media_file_path TEXT,
    workspace_dir_path TEXT,
    frame_dir_path TEXT,
    sidecar_file_path TEXT,
    started_at TEXT NOT NULL CHECK (LENGTH(TRIM(started_at)) > 0),
    ended_at TEXT NOT NULL CHECK (LENGTH(TRIM(ended_at)) > 0),
    status TEXT NOT NULL CHECK (status IN ('recording', 'completed', 'failed', 'pending_delete')),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(source_kind, source_session_id, segment_index)
);

CREATE INDEX IF NOT EXISTS capture_segments_capture_session_idx
    ON capture_segments(capture_session_id, id);
CREATE INDEX IF NOT EXISTS capture_segments_source_idx
    ON capture_segments(source_kind, source_session_id, segment_index, id);
CREATE INDEX IF NOT EXISTS capture_segments_ended_at_idx
    ON capture_segments(ended_at, id);

ALTER TABLE frames ADD COLUMN capture_segment_id INTEGER REFERENCES capture_segments(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS frames_capture_segment_id_idx ON frames(capture_segment_id, id);

ALTER TABLE audio_segments ADD COLUMN capture_segment_id INTEGER REFERENCES capture_segments(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS audio_segments_capture_segment_id_idx ON audio_segments(capture_segment_id, id);

CREATE TABLE IF NOT EXISTS retention_cleanup_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    policy TEXT NOT NULL CHECK (policy IN ('never', 'days_7', 'days_14', 'days_30')),
    mode TEXT NOT NULL CHECK (mode IN ('dry_run', 'manual', 'automatic', 'retry')),
    cutoff_started_at TEXT,
    cutoff_ended_before TEXT,
    status TEXT NOT NULL CHECK (status IN ('skipped', 'completed', 'completed_with_file_errors', 'failed')),
    deleted_capture_segments INTEGER NOT NULL DEFAULT 0,
    deleted_frames INTEGER NOT NULL DEFAULT 0,
    deleted_audio_segments INTEGER NOT NULL DEFAULT 0,
    deleted_processing_jobs INTEGER NOT NULL DEFAULT 0,
    deleted_processing_results INTEGER NOT NULL DEFAULT 0,
    deleted_background_jobs INTEGER NOT NULL DEFAULT 0,
    deleted_frame_batches INTEGER NOT NULL DEFAULT 0,
    deleted_speaker_rows INTEGER NOT NULL DEFAULT 0,
    skipped_running_jobs INTEGER NOT NULL DEFAULT 0,
    skipped_active_segments INTEGER NOT NULL DEFAULT 0,
    pending_file_tombstones INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS retention_cleanup_runs_created_at_idx
    ON retention_cleanup_runs(created_at, id);

CREATE TABLE IF NOT EXISTS retention_file_tombstones (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    cleanup_run_id INTEGER REFERENCES retention_cleanup_runs(id) ON DELETE SET NULL,
    capture_segment_id INTEGER,
    path TEXT NOT NULL CHECK (LENGTH(TRIM(path)) > 0),
    path_kind TEXT NOT NULL CHECK (path_kind IN ('media_file', 'workspace_dir', 'frame_dir', 'sidecar_file')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'resolved', 'failed')),
    last_error TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    resolved_at TEXT
);

CREATE INDEX IF NOT EXISTS retention_file_tombstones_status_idx
    ON retention_file_tombstones(status, id);
