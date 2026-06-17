-- The hidden-segment-workspace repair scan classifies every leftover workspace
-- by prefix-matching frame paths (`frames.file_path` under a workspace dir). With
-- no index on `file_path`, each per-workspace lookup was a full table scan of
-- `frames`, so a single 5-minute repair pass ran hundreds of full scans against a
-- multi-GB table, starving the live capture writer ("database is locked") and
-- bloating the WAL. The classifier queries now use range bounds
-- (`file_path >= ?1 AND file_path < ?2`) which this index turns into a cheap
-- index range search.
CREATE INDEX IF NOT EXISTS frames_file_path_idx ON frames (file_path);
