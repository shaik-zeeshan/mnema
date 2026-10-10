-- Pinned chats: a user flag that sorts a conversation into a "Pinned" group at
-- the top of the history list and search results. Distinct from the per-chat
-- ENGINE pin (`provider`/`model`, 0033). Setting it never touches the activity
-- stamps, and retention ignores it: a pinned chat ages out like any other.
ALTER TABLE conversations ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
