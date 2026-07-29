-- One-shot deletion of every speaker-recognition rejection written under the old
-- embedding-similarity blacklist, which suppressed a person globally and
-- permanently for any similar voice. The rows cannot be triaged (no origin
-- column: a genuine "not this person" is byte-identical to a casual unlink), so
-- they all go.
--
-- This lives in a migration, not in startup maintenance: maintenance runs on the
-- deferred-startup thread *after* the window (and every speaker command) is
-- live, so an unconditional DELETE there also eats a rejection the user makes
-- while the maintenance scans are still running. Migrations run inside
-- `Database::initialize`, before any pool is handed to the app, and sqlx's
-- `_sqlx_migrations` ledger is the one-shot guard.
DELETE FROM speaker_recognition_rejections;

-- Every new read path keys on `source_cluster_id` alone (the per-cluster veto,
-- the re-analysis purge guard, the retention GC), which neither the auto-index
-- (leading column `person_id`) nor `idx_speaker_recognition_rejections_model`
-- can serve.
CREATE INDEX IF NOT EXISTS idx_speaker_recognition_rejections_source_cluster
    ON speaker_recognition_rejections(source_cluster_id, person_id);
