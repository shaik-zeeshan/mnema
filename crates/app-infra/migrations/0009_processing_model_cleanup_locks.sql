CREATE TABLE IF NOT EXISTS processing_model_cleanup_locks (
    processor TEXT NOT NULL CHECK (LENGTH(TRIM(processor)) > 0),
    model_key TEXT NOT NULL CHECK (LENGTH(TRIM(model_key)) > 0),
    lock_token TEXT NOT NULL CHECK (LENGTH(TRIM(lock_token)) > 0),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (processor, model_key)
);

CREATE INDEX IF NOT EXISTS processing_model_cleanup_locks_token_idx
    ON processing_model_cleanup_locks (lock_token);
