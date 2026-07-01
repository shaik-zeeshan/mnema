-- Clearing the stale equivalent-reuse projections produced by a source frame's
-- OCR result deletes from `search_documents` by both `processing_result_id` and
-- `text_source_kind`:
--   DELETE FROM search_documents
--   WHERE processing_result_id = ? AND text_source_kind = 'equivalent_reuse'
-- The single-column `search_documents_result_idx (processing_result_id)` was not
-- chosen by the planner for this shape, so the delete full-scanned the (now
-- multi-million-row) table — a ~2.5s writer-lock hold on every OCR completion,
-- even when nothing matched. This composite index covers both predicates so the
-- delete becomes a cheap index seek.
CREATE INDEX IF NOT EXISTS search_documents_result_kind_idx
    ON search_documents (processing_result_id, text_source_kind);
