-- The subject vector worker polls `list_subjects_needing_embedding` every 30 s.
-- That read scanned every Conclusion's full row (statement text included) and,
-- per Subject, its vector row. `embedded_model` was added by 0044, so it sits
-- after the ~3 KB `embedding` BLOB, and comparing it walked that blob's overflow
-- pages. Each capture commit resets the reader's page cache, so every poll
-- re-read all of it: ~12 MB a poll, ~3 GB of disk reads in a 2-hour run
-- (stmt-cost trace, 2026-10-19). These two indexes answer the poll from index
-- entries alone.
CREATE INDEX IF NOT EXISTS user_context_conclusions_subject_status_idx
    ON user_context_conclusions (subject COLLATE NOCASE, status);
CREATE INDEX IF NOT EXISTS user_context_subject_vectors_embedded_idx
    ON user_context_subject_vectors (embedded_model, subject COLLATE NOCASE);
