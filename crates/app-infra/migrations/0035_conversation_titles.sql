-- Generated + user-set conversation titles (ADR 0034 chat-rail upgrade).
--
-- Two nullable columns on the `conversations` row, layered ABOVE the legacy
-- `title` column (which keeps holding the frontend's first-question truncation
-- on upsert):
--   generated_title — a short model-generated title, written ONCE after the
--                     thread's first turn completes, and only while BOTH
--                     columns are still NULL (a conditional UPDATE, so a racing
--                     user rename always wins)
--   user_title      — an explicit user rename; once set it wins FOREVER (the
--                     generator never overwrites it)
-- Read-path precedence: user_title → generated_title → title → first-question
-- preview truncation. Both columns OBEY retention/wipe with the rest of the row.
ALTER TABLE conversations ADD COLUMN generated_title TEXT;
ALTER TABLE conversations ADD COLUMN user_title TEXT;
