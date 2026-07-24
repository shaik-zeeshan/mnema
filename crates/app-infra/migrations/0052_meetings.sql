-- The detected-meetings ledger (Warm Paper redesign, Slice 1).
--
-- One row per detected meeting (ADR 0057 mic-hold), written by the meeting
-- detector worker WHETHER OR NOT a recap trigger fires — the Meetings surface
-- reads this, so a meeting with no trigger still appears (transcript-only).
--
-- `id` is deterministic (`meeting-<start_ms>-<bundle_id>`) so re-observation is
-- idempotent (INSERT OR IGNORE). `recap_state` is the decision-time verdict:
--   'none'    — no recap trigger fired (transcript-only)
--   'pending' — a firing was spawned; the final outcome resolves at read time
--               against `trigger_firings` (via `conversation_id` for
--               completed/failed rows, or `trigger_id` + `fired_at_ms` for
--               readiness-skip rows — the run path stamps its own fired_at)
--   'skipped' — dropped at decision time (cooldown / no provider), with reason
-- `trigger_id` references `triggers.json` (config, not DB) — no FK, like
-- `trigger_firings`. `notes` is the user's own text on the meeting detail.

CREATE TABLE meetings (
    id TEXT PRIMARY KEY,
    bundle_id TEXT NOT NULL,
    app_display_name TEXT NOT NULL,
    meeting_url TEXT,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER NOT NULL,
    trigger_id TEXT,
    fired_at_ms INTEGER,
    conversation_id TEXT,
    recap_state TEXT NOT NULL DEFAULT 'none'
        CHECK (recap_state IN ('none', 'pending', 'skipped')),
    recap_reason TEXT,
    notes TEXT
);

CREATE INDEX meetings_start_idx ON meetings (start_ms DESC);
