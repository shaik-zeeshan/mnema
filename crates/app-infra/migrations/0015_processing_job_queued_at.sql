ALTER TABLE processing_jobs
ADD COLUMN queued_at TEXT;

UPDATE processing_jobs
SET queued_at = created_at
WHERE queued_at IS NULL;
