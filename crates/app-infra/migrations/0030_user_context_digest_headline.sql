-- User Context **Digests** (migration 0029): the Insights lede gains a generated
-- **headline** — a short, evocative-but-factual title rendered in large type
-- above the narrative prose (e.g. "A deep week in the editor"). NULL on rows
-- generated before this column existed; the `digest_input_fingerprint` version
-- bump (`v2:` in `user_context/store.rs`) invalidates every cached digest, so
-- pre-headline narratives regenerate WITH a headline on next view rather than
-- needing a backfill here.

ALTER TABLE user_context_digests ADD COLUMN headline TEXT;
