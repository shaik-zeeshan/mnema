CREATE TABLE IF NOT EXISTS person_profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    display_name TEXT NOT NULL,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS person_voice_embeddings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    person_id INTEGER NOT NULL REFERENCES person_profiles(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    model_id TEXT NOT NULL,
    embedding BLOB NOT NULL,
    source_session_id TEXT,
    source_cluster_id INTEGER,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS recording_speaker_clusters (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    model_id TEXT,
    provider_cluster_id TEXT NOT NULL,
    stable_label TEXT NOT NULL,
    person_id INTEGER REFERENCES person_profiles(id) ON DELETE SET NULL,
    transcript_local_label TEXT,
    recognition_person_id INTEGER REFERENCES person_profiles(id) ON DELETE SET NULL,
    recognition_confidence TEXT,
    recognition_score REAL,
    embedding BLOB,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(session_id, provider, provider_cluster_id)
);

CREATE TABLE IF NOT EXISTS speaker_turns (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    audio_segment_id INTEGER NOT NULL REFERENCES audio_segments(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    cluster_id INTEGER NOT NULL REFERENCES recording_speaker_clusters(id) ON DELETE CASCADE,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER NOT NULL,
    transcript_text TEXT,
    overlaps INTEGER NOT NULL DEFAULT 0,
    moved_to_cluster_id INTEGER REFERENCES recording_speaker_clusters(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS speaker_cluster_merges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    source_cluster_id INTEGER NOT NULL REFERENCES recording_speaker_clusters(id) ON DELETE CASCADE,
    target_cluster_id INTEGER NOT NULL REFERENCES recording_speaker_clusters(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(source_cluster_id, target_cluster_id)
);

CREATE INDEX IF NOT EXISTS idx_person_voice_embeddings_person
    ON person_voice_embeddings(person_id);
CREATE INDEX IF NOT EXISTS idx_recording_speaker_clusters_session
    ON recording_speaker_clusters(session_id);
CREATE INDEX IF NOT EXISTS idx_speaker_turns_segment
    ON speaker_turns(audio_segment_id, start_ms, end_ms);
CREATE INDEX IF NOT EXISTS idx_speaker_turns_session
    ON speaker_turns(session_id, start_ms, end_ms);
