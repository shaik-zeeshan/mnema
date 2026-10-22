-- When an Activity was last extended by Activity continuation (NULL = never
-- extended since insert). Conclusion distillation reads
-- MAX(created_at_ms, updated_at_ms) as the row's change stamp so a pass sends
-- only Activities inserted or extended since the last successful pass.
ALTER TABLE user_context_activities ADD COLUMN updated_at_ms INTEGER;
