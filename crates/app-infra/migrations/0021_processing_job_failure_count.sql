-- Failure attempts, tracked distinctly from the total `attempt_count`.
-- `attempt_count` counts every run (including a reclaimed re-run after the app was
-- quit or crashed mid-job), and gates the reclamation abandonment ceiling.
-- `failure_count` counts only genuine failures (engine error, malformed output, the
-- speaker-helper subprocess timeout), and gates the bounded failure retry cap. A job
-- abandoned at quit/crash is requeued by Processing Job Reclamation without spending a
-- failure attempt, so repeated quits never exhaust the failure cap. Existing rows
-- default to 0.
ALTER TABLE processing_jobs
ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0;
