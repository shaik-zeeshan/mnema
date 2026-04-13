CREATE TABLE IF NOT EXISTS frame_batches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL CHECK (LENGTH(TRIM(session_id)) > 0),
    batch_key TEXT NOT NULL CHECK (LENGTH(TRIM(batch_key)) > 0),
    batch_started_at TEXT NOT NULL CHECK (LENGTH(TRIM(batch_started_at)) > 0),
    batch_ended_at TEXT NOT NULL CHECK (LENGTH(TRIM(batch_ended_at)) > 0),
    status TEXT NOT NULL CHECK (status IN ('open', 'closed', 'processing', 'completed', 'failed')),
    frame_count INTEGER NOT NULL DEFAULT 0,
    first_frame_at TEXT,
    last_frame_at TEXT,
    combine_job_id INTEGER REFERENCES background_jobs (id) ON DELETE SET NULL,
    combined_video_path TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    closed_at TEXT,
    completed_at TEXT,
    failed_at TEXT,
    last_error TEXT,
    UNIQUE (session_id, batch_key)
);

ALTER TABLE frames ADD COLUMN frame_batch_id INTEGER REFERENCES frame_batches (id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS frame_batches_session_status_idx
    ON frame_batches (session_id, status, id DESC);
CREATE INDEX IF NOT EXISTS frame_batches_combine_job_idx
    ON frame_batches (combine_job_id);
CREATE INDEX IF NOT EXISTS frames_frame_batch_id_idx
    ON frames (frame_batch_id, id ASC);
