-- The Trigger firing ledger (ADR 0058, issue #176).
--
-- One row per Firing decision: completed (with the run's conversation link),
-- skipped (nothing to work with), or failed (the AI run did not complete after
-- retries). Good-news-only delivery reads this nowhere — notifications fire
-- only on completed — but the Triggers page's last-run status and the persisted
-- Cooldown both do. `trigger_id` references `triggers.json` (config, not DB —
-- ADR 0058) across the file/DB boundary, deliberately with NO foreign key;
-- deleting a trigger deletes its rows by id.

CREATE TABLE trigger_firings (
    trigger_id TEXT NOT NULL,
    fired_at_ms INTEGER NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('completed', 'skipped', 'failed')),
    reason TEXT,
    conversation_id TEXT
);

CREATE INDEX trigger_firings_trigger_fired_idx
    ON trigger_firings (trigger_id, fired_at_ms);
