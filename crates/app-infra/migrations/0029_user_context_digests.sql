-- User Context **Digests**: the Insights Overview's engine-written narrative
-- lede — one short prose paragraph ("the story this week") per
-- (range_kind, range_start_ms) range, derived from that range's Activities.
-- Generation is lazy in the Tauri layer; this table only stores the result plus
-- the deterministic `input_fingerprint` of the Activities it was derived from,
-- so a stale digest (range edited by new derivation / correction / deletion) is
-- detected by fingerprint mismatch and regenerated.
--
-- Deliberately NO foreign keys to frames/audio_segments (ADR 0029): derived
-- data must SURVIVE Retention Policy aging. Delete Recent Capture purges
-- overlapping digests explicitly via the cascade in `user_context/store.rs`.
--
-- Timestamp convention (same as 0022..0027): INTEGER unix milliseconds columns
-- named `*_at_ms` / `*_ms`, set from Rust at write (NOT CURRENT_TIMESTAMP).

CREATE TABLE IF NOT EXISTS user_context_digests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    range_kind TEXT NOT NULL,             -- 'day' | 'week' | 'month'
    range_start_ms INTEGER NOT NULL,
    range_end_ms INTEGER NOT NULL,        -- exclusive: [range_start_ms, range_end_ms)
    narrative TEXT NOT NULL,
    input_fingerprint TEXT NOT NULL,
    generated_at_ms INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS user_context_digests_range_idx
    ON user_context_digests (range_kind, range_start_ms);
