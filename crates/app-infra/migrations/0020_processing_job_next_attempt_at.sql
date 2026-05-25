-- Retry backoff for failed processing jobs. When a failed OCR job is requeued
-- within its attempt cap, `next_attempt_at` holds the earliest wall-clock time
-- it may be re-claimed by the automatic queue drain. A NULL value means the job
-- is immediately eligible. Stored in the same 'YYYY-MM-DD HH:MM:SS' UTC format
-- as `created_at` / `queued_at` so it compares lexically against CURRENT_TIMESTAMP.
ALTER TABLE processing_jobs
ADD COLUMN next_attempt_at TEXT;
