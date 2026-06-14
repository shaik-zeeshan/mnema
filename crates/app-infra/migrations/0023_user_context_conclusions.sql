-- User Context (issue #88) Conclusion layer (issue #94): distilled, plain-language
-- beliefs about the user, each grounded in the Activity evidence it points back to.
--
-- Timestamp convention (same as 0022): INTEGER unix milliseconds columns named
-- `*_at_ms`, set from Rust at insert (NOT CURRENT_TIMESTAMP). Confidence is a REAL
-- in [0.0, 1.0]. A Conclusion is open-ended natural language (subject + statement),
-- NOT a fixed subject+attribute+value row.

CREATE TABLE IF NOT EXISTS user_context_conclusions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject TEXT NOT NULL,
    statement TEXT NOT NULL,
    confidence REAL NOT NULL,
    status TEXT NOT NULL DEFAULT 'visible',     -- 'visible' | 'faded' | 'dismissed'
    formed_at_ms INTEGER NOT NULL,
    last_supported_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS user_context_conclusions_subject_idx
    ON user_context_conclusions (subject);
CREATE INDEX IF NOT EXISTS user_context_conclusions_status_idx
    ON user_context_conclusions (status, confidence);

-- Evidence: Conclusion -> the Activity values that support (or contradict) it.
-- Cascades from both sides: dropping an Activity (e.g. Delete Recent Capture) or a
-- Conclusion drops the link rows. A Conclusion that loses ALL its evidence is dropped
-- by the cascade (#97) so there are no ungrounded Conclusions.
CREATE TABLE IF NOT EXISTS user_context_conclusion_evidence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conclusion_id INTEGER NOT NULL REFERENCES user_context_conclusions(id) ON DELETE CASCADE,
    activity_id INTEGER NOT NULL REFERENCES user_context_activities(id) ON DELETE CASCADE,
    stance TEXT NOT NULL DEFAULT 'support',     -- 'support' | 'contradict'
    created_at_ms INTEGER NOT NULL,
    UNIQUE (conclusion_id, activity_id)
);
CREATE INDEX IF NOT EXISTS user_context_conclusion_evidence_conclusion_idx
    ON user_context_conclusion_evidence (conclusion_id);
CREATE INDEX IF NOT EXISTS user_context_conclusion_evidence_activity_idx
    ON user_context_conclusion_evidence (activity_id);
