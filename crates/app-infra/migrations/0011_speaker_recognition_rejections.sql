CREATE TABLE IF NOT EXISTS speaker_recognition_rejections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    person_id INTEGER NOT NULL REFERENCES person_profiles(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    model_id TEXT NOT NULL,
    embedding BLOB NOT NULL,
    source_session_id TEXT,
    source_cluster_id INTEGER REFERENCES recording_speaker_clusters(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(person_id, provider, model_id, source_cluster_id)
);

CREATE INDEX IF NOT EXISTS idx_speaker_recognition_rejections_model
    ON speaker_recognition_rejections(provider, model_id, person_id);
