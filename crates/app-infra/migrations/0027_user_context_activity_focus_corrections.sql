-- User Context (issue #88): Activity Focus Classification (issue #105) + the
-- Category/Focus correction feedback loop (issue #108).
--
-- #105 adds a per-Activity Focus Classification alongside the existing
-- `category` column (migration 0022): a small fixed taxonomy ('deep' | 'mixed'
-- | 'distracted', the capture-types `FocusLevel` snake_case rename) the engine
-- assigns per Activity to drive the Overview focus/distraction heatmap. NULL
-- when the engine omitted it.
--
-- #108 records user CORRECTIONS as override columns on the activities row
-- itself (not a side table): the cleaner option here because a correction is
-- per-row state read on every Activity load — exactly like the existing
-- `pinned` / `last_decayed_at_ms` override columns on user_context_conclusions.
-- A side table would force a LEFT JOIN on every read for a strictly 1:1 fact.
--
-- The user override always WINS over the engine label: reads coalesce the
-- corrected value when its `*_corrected` flag is set, else fall back to the
-- engine column. The `*_corrected` flag is required (not "corrected = NULL ?")
-- because a user may deliberately correct a label to "unset" (NULL), which must
-- be distinguished from "never corrected". `corrected_at_ms` stamps the most
-- recent correction (the feedback loop feeds corrected activities back into the
-- next derivation prompt so the engine is biased away from regenerating the
-- corrected-away label).
--
-- Timestamp convention (same as 0022..0026): INTEGER unix milliseconds, set
-- from Rust at write (NOT CURRENT_TIMESTAMP).

-- Engine-assigned Focus Classification ('deep' | 'mixed' | 'distracted'), NULL
-- when the engine omitted it.
ALTER TABLE user_context_activities ADD COLUMN focus TEXT;

-- #108 correction overrides. The `*_corrected` flags distinguish "user
-- corrected this to NULL" from "never corrected"; the `corrected_*` columns
-- hold the user's chosen value (which may itself be NULL when the flag is set).
ALTER TABLE user_context_activities ADD COLUMN corrected_category TEXT;
ALTER TABLE user_context_activities ADD COLUMN category_corrected INTEGER NOT NULL DEFAULT 0;
ALTER TABLE user_context_activities ADD COLUMN corrected_focus TEXT;
ALTER TABLE user_context_activities ADD COLUMN focus_corrected INTEGER NOT NULL DEFAULT 0;
ALTER TABLE user_context_activities ADD COLUMN corrected_at_ms INTEGER;
