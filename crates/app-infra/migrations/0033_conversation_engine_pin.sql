-- Per-conversation engine PIN (Chat thread engine identity).
--
-- A Chat (or Quick Recall) thread can pin a specific Reasoning Engine identity
-- so its turns always run against that engine instead of the global default.
-- The pin is two nullable columns on the `conversations` row:
--   provider — the engine provider id (e.g. 'anthropic' | 'openai' | 'ollama')
--   model    — the model id within that provider
-- A later slice resolves `{provider, model}` to a concrete engine. Both NULL
-- (the default, no DEFAULT clause) means UNPINNED → use the global default
-- engine. The pin is metadata only; it OBEYS retention with the rest of the row.
ALTER TABLE conversations ADD COLUMN provider TEXT;
ALTER TABLE conversations ADD COLUMN model TEXT;
