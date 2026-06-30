-- Semantic Search Vector storage substrate.
--
-- One Semantic Search Vector per Search Result Anchor (search_documents row),
-- stored inside the Encrypted Capture Index. `vec0` is sqlite-vec's virtual
-- table, statically linked into the same SQLCipher amalgamation, so vectors
-- live encrypted-at-rest alongside the FTS5 projection.
--
-- The vec0 table stays a pure {rowid, embedding} store keyed to
-- search_documents.id: all scoping is filter-then-rank against search_documents
-- (ADR 0036), so there is no metadata to mirror here. Embeddings are stored as
-- int8: the embedder L2-normalizes (unit vectors), so the write/query paths feed
-- each f32 vector through `vec_quantize_int8(?, 'unit')` (4× smaller at rest) and
-- unit-vector L2 ordering ≡ cosine ordering, so KNN rank is unchanged with no
-- rescore. The 768-dim column matches the English default tier
-- (nomic-embed-text-v1.5); changing the Semantic Search Model Tier re-derives
-- every vector behind a confirm, so a fixed dimension here is intentional.
CREATE VIRTUAL TABLE IF NOT EXISTS search_document_vectors USING vec0(
    embedding int8[768]
);

-- Deletion flows through one AFTER DELETE trigger, a near-copy of
-- search_documents_fts_after_delete (0016): when a Search Result Anchor is
-- removed (retention, Delete Recent, reprocess — including CASCADE-driven frame
-- deletes), its Semantic Search Vector is dropped in lockstep so lifecycle stays
-- nearly free. vec0 rows key off the rowid that equals search_documents.id.
CREATE TRIGGER IF NOT EXISTS search_document_vectors_after_delete
AFTER DELETE ON search_documents
BEGIN
    DELETE FROM search_document_vectors WHERE rowid = OLD.id;
END;
