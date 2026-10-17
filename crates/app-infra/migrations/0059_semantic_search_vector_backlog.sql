-- The Semantic Index Backfill's work list: one row per `direct` Search Result
-- Anchor that still lacks a Semantic Search Vector.
--
-- Before this, every worker pass found that set with
-- `id NOT IN (SELECT rowid FROM search_document_vectors)`, and both halves of it
-- scaled with the whole index, not the backlog:
--   - vec0 answers `SELECT rowid` with a full scan of its `_rowids` shadow table
--     plus a sort, rebuilt for EVERY statement — the peek, each of the per-anchor
--     re-checks, and the count;
--   - live capture keeps the backlog below the peek's `LIMIT 16`, so the peek
--     never stopped early and walked every `direct` anchor in
--     `search_documents_direct_recent_idx` to prove there was nothing more.
-- Each capture commit resets the readers' page cache, so every pass re-read all of
-- it from the file: ~210 MB a pass on a 970 MB install (fs_usage, 2026-10-17).
--
-- This table is what the peek walks instead, newest-first through its own index,
-- so a pass reads pages in proportion to the backlog. It is maintained entirely in
-- SQL, in the same transaction as the change it mirrors: the insert trigger below
-- (every `direct` anchor starts missing), the delete trigger (a deleted anchor
-- needs no vector), the store (which deletes the row in the transaction that
-- writes the vector), and the vec0 rebuild (which refills it). `absolute_start_at`
-- is copied so the newest-first order needs no join; nothing ever UPDATEs it.
CREATE TABLE IF NOT EXISTS search_document_vector_backlog (
    id INTEGER PRIMARY KEY,
    absolute_start_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS search_document_vector_backlog_recent_idx
    ON search_document_vector_backlog (absolute_start_at DESC, id DESC);

INSERT OR IGNORE INTO search_document_vector_backlog (id, absolute_start_at)
SELECT id, absolute_start_at
FROM search_documents
WHERE text_source_kind = 'direct'
  AND id NOT IN (SELECT rowid FROM search_document_vectors);

CREATE TRIGGER IF NOT EXISTS search_document_vector_backlog_after_insert
AFTER INSERT ON search_documents
WHEN NEW.text_source_kind = 'direct'
BEGIN
    INSERT OR IGNORE INTO search_document_vector_backlog (id, absolute_start_at)
    VALUES (NEW.id, NEW.absolute_start_at);
END;

CREATE TRIGGER IF NOT EXISTS search_document_vector_backlog_after_delete
AFTER DELETE ON search_documents
WHEN OLD.text_source_kind = 'direct'
BEGIN
    DELETE FROM search_document_vector_backlog WHERE id = OLD.id;
END;
