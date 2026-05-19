# Search v1 uses local text search in the app database

Mnema will ship search v1 as local **Text Search** backed by SQLite FTS5 inside the active app-infra database. This keeps search projections transactionally aligned with completed OCR/transcription results, retention cleanup, and the current `saveDirectory` boundary, while avoiding a second search store before the core result/navigation behavior is proven.

Semantic embeddings and **Hybrid Search** remain product direction, but they are deferred until after text search works end to end. When added, semantic indexing must remain local-only and should run as separate model work rather than blocking OCR or transcription completion.

## Alternatives Rejected

- Separate search engine/index directory: adds cross-store consistency, backup, and retention-delete failure modes before we need them.
- Vector-first search: weak for literal screen/audio queries such as URLs, code, errors, app names, and exact phrases.
- Cloud embeddings: conflicts with Mnema's local-first treatment of captured screen and audio content.
