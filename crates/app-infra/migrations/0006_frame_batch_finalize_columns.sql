-- Additive migration: introduce finalization-oriented column names for
-- frame_batches.  The old combine_job_id / combined_video_path columns are
-- kept (SQLite cannot drop columns portably) but all new reads/writes target
-- the new columns only.
--
-- Data from existing rows is copied so upgraded databases keep working.

ALTER TABLE frame_batches ADD COLUMN finalize_job_id INTEGER REFERENCES background_jobs (id) ON DELETE SET NULL;
ALTER TABLE frame_batches ADD COLUMN finalized_output_path TEXT;

UPDATE frame_batches SET finalize_job_id = combine_job_id, finalized_output_path = combined_video_path
    WHERE combine_job_id IS NOT NULL OR combined_video_path IS NOT NULL;

CREATE INDEX IF NOT EXISTS frame_batches_finalize_job_idx
    ON frame_batches (finalize_job_id);
