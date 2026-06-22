-- Partial index for the Semantic Index Backfill sweep (F6).
--
-- `anchors_missing_vector` / `count_anchors_missing_vector` (and the worker's peek)
-- filter `WHERE text_source_kind = 'direct' AND id NOT IN (SELECT rowid FROM
-- search_document_vectors)`, and the peek then `ORDER BY absolute_start_at DESC,
-- id DESC LIMIT 16` so the newest un-vectored anchors drain first (ADR 0036).
--
-- No existing index leads with `text_source_kind`, so each call was a full scan of
-- search_documents plus a correlated anti-join and a filesort for the ORDER BY.
-- This partial index covers exactly the `direct` rows in the ORDER BY's leading
-- key order, so:
--   - the peek is index-driven and `LIMIT 16` short-circuits after 16 rows instead
--     of sorting the whole `direct` set;
--   - the `text_source_kind = 'direct'` predicate is satisfied by the partial
--     index's WHERE clause (the index only holds `direct` rows), so the scan never
--     touches `equivalent_reuse` rows;
--   - the count's filter narrows to the index too.
-- The `NOT IN (search_document_vectors)` anti-join still probes the vec0 rowids,
-- but it now runs over the small index-ordered candidate stream rather than a full
-- table scan.
--
-- Partial (`WHERE text_source_kind = 'direct'`) keeps the index small — only the
-- embeddable anchors are indexed, not the whole projection — and matches the
-- query's constant predicate so SQLite can use it.
CREATE INDEX IF NOT EXISTS search_documents_direct_recent_idx
    ON search_documents (absolute_start_at DESC, id DESC)
    WHERE text_source_kind = 'direct';
