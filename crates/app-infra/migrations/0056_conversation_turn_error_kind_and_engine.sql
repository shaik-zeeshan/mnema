-- A failed turn keeps its error KIND (`TurnUpdate::Error.kind`: reconnect |
-- unreachable | settings | quota | auth | context_too_long | retryable) so a
-- reloaded chat offers the same action as the live one did. And each turn
-- records the engine that wrote it (`provider` instance id + `model`), stamped
-- when the engine resolves at the start of the turn, so switching models
-- mid-chat never relabels older answers. All nullable: old turns read NULL
-- (no kind → generic error; no engine → the chat's current model).
ALTER TABLE conversation_turns ADD COLUMN error_kind TEXT;
ALTER TABLE conversation_turns ADD COLUMN provider TEXT;
ALTER TABLE conversation_turns ADD COLUMN model TEXT;
