-- Persistent Quick Recall / Chat conversations (issue #102, ADR 0031).
--
-- ONE shared conversation store backs both doors (Quick Recall and Chat). These
-- live in the Encrypted Capture Index so conversations persist across restarts.
-- Unlike the User Context dossier (which OUTLIVES retention), conversations OBEY
-- Retention Policy: they are aged out by the same local-calendar cutoff capture
-- cleanup uses (driven by `last_activity_at_ms`), and are CLEARED by Wipe User
-- Context.
--
-- IMPORTANT: these tables are deliberately NOT prefixed `user_context_` — the
-- retention cleanup source DOES name the conversations table (to age old
-- conversations out), and a structural test asserts retention never names a
-- `user_context_*` table.
--
-- Timestamp convention (same as the rest of the User Context tables): INTEGER
-- unix milliseconds columns named `*_at_ms`, set from Rust at insert (NOT
-- CURRENT_TIMESTAMP).

CREATE TABLE IF NOT EXISTS conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL UNIQUE,           -- frontend-generated UUID
    title TEXT NOT NULL DEFAULT '',
    origin TEXT NOT NULL DEFAULT 'quick_recall',    -- 'quick_recall' | 'chat'
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    last_activity_at_ms INTEGER NOT NULL            -- drives retention aging
);
CREATE INDEX IF NOT EXISTS conversations_last_activity_idx
    ON conversations (last_activity_at_ms);
CREATE INDEX IF NOT EXISTS conversations_updated_idx
    ON conversations (updated_at_ms);

CREATE TABLE IF NOT EXISTS conversation_turns (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_row_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    turn_index INTEGER NOT NULL,
    question TEXT NOT NULL,
    answer TEXT NOT NULL DEFAULT '',
    tool_activities TEXT NOT NULL DEFAULT '[]',     -- JSON array
    sources TEXT NOT NULL DEFAULT '[]',             -- JSON array
    phase TEXT NOT NULL DEFAULT 'streaming',
    error_message TEXT,
    seeded_result_count INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (conversation_row_id, turn_index)
);
CREATE INDEX IF NOT EXISTS conversation_turns_conversation_idx
    ON conversation_turns (conversation_row_id, turn_index);
