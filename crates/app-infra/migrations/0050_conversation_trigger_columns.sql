-- Trigger Runs are conversations (ADR 0058, issue #175).
--
-- A Trigger Run persists as a NORMAL conversation in the existing store: the
-- existing `origin` column gains a new value `'trigger'` (alongside
-- 'quick_recall' / 'chat'), and these two columns carry the firing trigger's
-- identity. The trigger *definition* lives in `triggers.json` (config, not DB —
-- ADR 0058), so `trigger_id` is an id string across the file/DB boundary with
-- deliberately NO foreign key; `trigger_name` is the display name snapshotted at
-- fire time so the conversation stays labeled even after the definition is
-- renamed or deleted. Both NULL for every non-trigger conversation.

ALTER TABLE conversations ADD COLUMN trigger_id TEXT;
ALTER TABLE conversations ADD COLUMN trigger_name TEXT;
