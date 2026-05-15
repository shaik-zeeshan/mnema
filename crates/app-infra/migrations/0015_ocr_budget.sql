CREATE TABLE IF NOT EXISTS frame_ocr_admissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    frame_id INTEGER NOT NULL UNIQUE REFERENCES frames(id) ON DELETE CASCADE,
    outcome TEXT NOT NULL CHECK (outcome IN ('admitted', 'skipped')),
    reason TEXT NOT NULL CHECK (reason IN (
        'admitted_initial',
        'admitted_context_change',
        'admitted_low_pressure',
        'admitted_representative',
        'skipped_equivalent_frame',
        'skipped_ocr_disabled',
        'skipped_provider_unavailable',
        'skipped_low_ocr_value'
    )),
    job_id INTEGER REFERENCES processing_jobs(id) ON DELETE SET NULL,
    related_frame_id INTEGER REFERENCES frames(id) ON DELETE SET NULL,
    queue_pressure_count INTEGER NOT NULL,
    recording_active INTEGER NOT NULL,
    signals_json TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS frame_ocr_admissions_reason_idx
    ON frame_ocr_admissions(reason);

CREATE INDEX IF NOT EXISTS frame_ocr_admissions_job_id_idx
    ON frame_ocr_admissions(job_id);

CREATE INDEX IF NOT EXISTS frame_ocr_admissions_related_frame_id_idx
    ON frame_ocr_admissions(related_frame_id);

CREATE TABLE IF NOT EXISTS ocr_budget_telemetry (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id INTEGER NOT NULL UNIQUE REFERENCES processing_jobs(id) ON DELETE CASCADE,
    frame_id INTEGER REFERENCES frames(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    model_id TEXT,
    recognition_mode TEXT,
    status TEXT NOT NULL CHECK (status IN ('completed', 'failed')),
    run_duration_ms INTEGER NOT NULL,
    queue_wait_ms INTEGER,
    result_text_length INTEGER,
    observation_count INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS ocr_budget_telemetry_frame_id_idx
    ON ocr_budget_telemetry(frame_id);
