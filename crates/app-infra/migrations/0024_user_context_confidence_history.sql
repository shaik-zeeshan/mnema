-- User Context (issue #88) Confidence History layer (issue #95): a stored
-- time-series of each Conclusion's Confidence — periodic snapshots, not just the
-- current value — that powers the Subject trajectory line. Tiny (a few floats per
-- snapshot) and aggressively prunable, since recency-weighting means old snapshots
-- stop mattering.
--
-- Timestamp convention (same as 0022/0023): INTEGER unix milliseconds columns
-- named `*_at_ms`, set from Rust at insert (NOT CURRENT_TIMESTAMP). Confidence is a
-- REAL in [0.0, 1.0].

CREATE TABLE IF NOT EXISTS user_context_confidence_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conclusion_id INTEGER NOT NULL REFERENCES user_context_conclusions(id) ON DELETE CASCADE,
    confidence REAL NOT NULL,
    snapshot_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS user_context_confidence_history_conclusion_idx
    ON user_context_confidence_history (conclusion_id, snapshot_at_ms);

-- Decay bookkeeping: the last wall-clock time the confidence-decay beat ran a
-- decay pass over a Conclusion. NULL until the first decay pass. The decay math
-- itself measures silence from `last_supported_at_ms`, not this column — this is a
-- record of when the beat last touched the row.
ALTER TABLE user_context_conclusions ADD COLUMN last_decayed_at_ms INTEGER;
