-- speaker_turns.cluster_id had no index, so the retention sweep's orphan
-- speaker-cluster GC (`DELETE FROM recording_speaker_clusters WHERE NOT EXISTS
-- (SELECT 1 FROM speaker_turns WHERE cluster_id = ...)`) planned as a correlated
-- full scan (~3.9k clusters x ~23k turns, measured 8.75s warm) inside every
-- batch write transaction — holding the writer lock long enough to starve every
-- other writer into "database is locked" for the whole sweep.
CREATE INDEX IF NOT EXISTS idx_speaker_turns_cluster
    ON speaker_turns(cluster_id);
