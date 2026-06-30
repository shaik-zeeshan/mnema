CREATE TABLE IF NOT EXISTS frames (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL CHECK (LENGTH(TRIM(session_id)) > 0),
    file_path TEXT NOT NULL CHECK (LENGTH(TRIM(file_path)) > 0),
    captured_at TEXT NOT NULL CHECK (LENGTH(TRIM(captured_at)) > 0),
    width INTEGER CHECK (width IS NULL OR width > 0),
    height INTEGER CHECK (height IS NULL OR height > 0),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (session_id, file_path)
);

CREATE INDEX IF NOT EXISTS frames_session_id_idx ON frames (session_id, id DESC);
CREATE INDEX IF NOT EXISTS frames_captured_at_idx ON frames (captured_at, id DESC);

CREATE TABLE IF NOT EXISTS processing_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL CHECK (LENGTH(TRIM(subject_type)) > 0),
    subject_id INTEGER NOT NULL,
    processor TEXT NOT NULL CHECK (LENGTH(TRIM(processor)) > 0),
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'failed')),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    payload_json TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at TEXT,
    finished_at TEXT
);

CREATE INDEX IF NOT EXISTS processing_jobs_subject_idx
    ON processing_jobs (subject_type, subject_id, id DESC);
CREATE INDEX IF NOT EXISTS processing_jobs_processor_idx
    ON processing_jobs (processor, status, id DESC);
CREATE INDEX IF NOT EXISTS processing_jobs_status_idx
    ON processing_jobs (status, id DESC);

CREATE TABLE IF NOT EXISTS processing_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL REFERENCES processing_jobs (id) ON DELETE CASCADE,
    subject_type TEXT NOT NULL CHECK (LENGTH(TRIM(subject_type)) > 0),
    subject_id INTEGER NOT NULL,
    processor TEXT NOT NULL CHECK (LENGTH(TRIM(processor)) > 0),
    result_text TEXT,
    -- OCR rows store a zstd-compressed JSON blob (geometry compression); every
    -- other processor stores plain JSON text. Decode keys off `processor`.
    structured_payload_json BLOB,
    processor_version TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (job_id)
);

CREATE INDEX IF NOT EXISTS processing_results_job_id_idx ON processing_results (job_id);
CREATE INDEX IF NOT EXISTS processing_results_subject_idx
    ON processing_results (subject_type, subject_id, id DESC);
CREATE INDEX IF NOT EXISTS processing_results_processor_idx
    ON processing_results (processor, id DESC);
