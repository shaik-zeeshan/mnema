-- Subject Vector storage for User Context (embedding-free in app-infra).
--
-- One row per Subject (the `user_context_conclusions.subject` handle). The
-- embedding itself is derived elsewhere (the Tauri layer owns the embedding
-- model); app-infra only persists the resulting f32 vector as a little-endian
-- byte BLOB and serves brute-force cosine lookups. NO embedding model lives in
-- this crate.
--
-- `subject` is a NOCASE primary key so it dedups case-insensitively in lockstep
-- with how Conclusions are matched (`subject = ?1 COLLATE NOCASE` throughout
-- `user_context/store.rs`): one vector per Subject regardless of casing.
--
-- `embedding` NULL means "stale / needs (re)embed" — the backfill worker claims
-- these via `list_subjects_without_vector`. `embedded_at_ms` is NULL until the
-- vector is written, and is cleared back to NULL when a Subject is marked stale.
CREATE TABLE IF NOT EXISTS user_context_subject_vectors (
    subject TEXT PRIMARY KEY COLLATE NOCASE,
    embedding BLOB,
    embedded_at_ms INTEGER
);
