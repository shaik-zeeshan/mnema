CREATE TABLE IF NOT EXISTS speaker_segment_clusters (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    audio_segment_id INTEGER NOT NULL REFERENCES audio_segments(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    model_id TEXT,
    provider_cluster_id TEXT NOT NULL,
    stable_cluster_id INTEGER REFERENCES recording_speaker_clusters(id) ON DELETE SET NULL,
    stable_label TEXT NOT NULL,
    embedding BLOB,
    embedding_model_id TEXT,
    suggested_merge_target_cluster_id INTEGER REFERENCES recording_speaker_clusters(id) ON DELETE SET NULL,
    suggested_merge_score REAL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(audio_segment_id, provider, provider_cluster_id)
);

ALTER TABLE speaker_turns ADD COLUMN segment_cluster_id INTEGER REFERENCES speaker_segment_clusters(id) ON DELETE SET NULL;
ALTER TABLE recording_speaker_clusters ADD COLUMN suggested_merge_target_cluster_id INTEGER REFERENCES recording_speaker_clusters(id) ON DELETE SET NULL;
ALTER TABLE recording_speaker_clusters ADD COLUMN suggested_merge_score REAL;

CREATE INDEX IF NOT EXISTS idx_speaker_segment_clusters_segment
    ON speaker_segment_clusters(audio_segment_id);
CREATE INDEX IF NOT EXISTS idx_speaker_segment_clusters_stable
    ON speaker_segment_clusters(stable_cluster_id);
CREATE INDEX IF NOT EXISTS idx_speaker_turns_segment_cluster
    ON speaker_turns(segment_cluster_id);
