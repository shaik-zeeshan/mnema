-- User Context Conclusion dedup hardening (#11): back the case-insensitive
-- (subject, statement) dedup with a real UNIQUE index so `upsert_conclusion` can
-- use `INSERT ... ON CONFLICT ... DO UPDATE` instead of a non-atomic
-- SELECT-then-INSERT. Migration 0023 created the table with NO unique constraint
-- on (subject, statement); under the `max_connections(4)` pool two concurrent
-- upserts of the same normalized pair both missed the SELECT and both INSERTed,
-- creating a duplicate the `ORDER BY id ASC LIMIT 1` dedup then hid forever (it
-- double-counted in recall_context).
--
-- NOCASE collation matches the rest of the store's ASCII-only case-insensitive
-- matching (the same `COLLATE NOCASE` the old dedup SELECT and
-- `list_conclusions_for_subject` use), so the index is the deterministic backing
-- for the existing dedup key — not a new one.
--
-- Existing duplicate rows (if any slipped in before this index) would block the
-- unique index, so collapse them first: keep the lowest id per normalized
-- (subject, statement) pair — exactly the row the old `ORDER BY id ASC LIMIT 1`
-- dedup already surfaced — and drop the rest. Their evidence / confidence-history
-- rows cascade via FK (ON DELETE CASCADE in 0023 / 0024).
DELETE FROM user_context_conclusions
WHERE id NOT IN (
    SELECT MIN(id)
    FROM user_context_conclusions
    GROUP BY lower(subject), lower(statement)
);

CREATE UNIQUE INDEX IF NOT EXISTS user_context_conclusions_subject_statement_unique_idx
    ON user_context_conclusions (subject COLLATE NOCASE, statement COLLATE NOCASE);
