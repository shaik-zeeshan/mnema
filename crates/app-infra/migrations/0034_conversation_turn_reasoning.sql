-- Per-turn reasoning (thinking) text on a conversation turn.
--
-- A conversation turn can carry the model's reasoning/"thinking" text alongside
-- its final `answer`. This is a single nullable column on the
-- `conversation_turns` row (no DEFAULT, so old turns read NULL → no reasoning).
-- It lives next to the other per-turn fields (`answer`, `tool_activities`,
-- `sources`, `phase`, `error_message`, `seeded_result_count`) and OBEYS retention
-- with the rest of the row.
ALTER TABLE conversation_turns ADD COLUMN reasoning TEXT;
