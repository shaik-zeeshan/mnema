-- User Context (issue #88) Dismiss / Pin + Dismissal State (issue #99).
--
-- Pin: a user correction meaning "this is true, keep it" — a pinned Conclusion is
-- exempt from Confidence decay so a quiet stretch does not quietly fade it. Stored
-- as a 0/1 flag on the Conclusion row (read by `map_conclusion`, excluded from
-- `list_decayable_conclusions`).
--
-- Dismissal State: engine-carried state recording that the user rejected a
-- particular Conclusion, with WHICH evidence and WHEN. Fed as input to every
-- derivation pass so the engine can tell *fresh* evidence from the evidence already
-- vetoed and honor the high-bar-resurface rule. A Dismiss removes the Conclusion
-- AND records this row — it is real state, not a hidden flag, and it deliberately
-- OUTLIVES the deleted Conclusion (no FK to it).
--
-- Timestamp convention (same as 0022..0024): INTEGER unix milliseconds columns
-- named `*_at_ms`, set from Rust at insert (NOT CURRENT_TIMESTAMP).

ALTER TABLE user_context_conclusions ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS user_context_dismissals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject TEXT NOT NULL,
    statement TEXT NOT NULL,
    -- Deterministic fingerprint of the evidence the dismissed Conclusion was built
    -- on: its sorted-distinct supporting Activity ids joined by ','. The resurface
    -- gate compares a freshly-distilled Conclusion's fingerprint to this so the same
    -- evidence just rejected can NEVER resurface the Conclusion.
    evidence_fingerprint TEXT NOT NULL,
    -- Count of support-stance evidence Activities at dismissal time, the baseline
    -- the high resurface bar is measured against (substantially MORE fresh support
    -- is required to overturn the Dismiss).
    evidence_activity_count INTEGER NOT NULL DEFAULT 0,
    dismissed_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS user_context_dismissals_subject_idx
    ON user_context_dismissals (subject);
