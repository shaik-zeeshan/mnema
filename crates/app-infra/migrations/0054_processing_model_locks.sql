-- The model-cleanup lock is no longer cleanup-only: a model that is downloading or
-- absent takes the same lock so the claim predicate parks its jobs instead of running
-- them against a model that is not on disk. Renamed additively (never edit 0009 in
-- place: an existing DB would trip sqlx VersionMismatch with no recovery path).
-- The token index rides along under its old name, which SQLite keeps attached to the
-- renamed table.
ALTER TABLE processing_model_cleanup_locks RENAME TO processing_model_locks;

-- 'cleanup' | 'downloading' | 'absent'. Existing rows are cleanup locks.
ALTER TABLE processing_model_locks ADD COLUMN reason TEXT NOT NULL DEFAULT 'cleanup';
