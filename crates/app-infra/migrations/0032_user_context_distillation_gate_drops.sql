-- User Context distillation observability: per-gate withheld counters on the
-- derivation-run ledger (kind 'conclusion' rows). Without these, a distillation
-- pass that produced nothing is indistinguishable from one whose drafts were all
-- withheld by policy, so a user asking "why is my dossier thin?" has no answer.
--
-- The four deterministic persist gates, in the order they run
-- (apps/desktop/src-tauri/src/user_context/derivation.rs::distill_conclusions):
--   ungrounded           -- engine draft with no resolvable supporting Activity
--   guardrail_suppressed -- Sensitive Category Guardrail hard post-filter (#96)
--   below_formation_bar  -- fewer supporting Activities than the bar (#95)
--   resurface_blocked    -- dismissed Conclusion not clearing the resurface bar (#99)
ALTER TABLE user_context_derivation_runs ADD COLUMN ungrounded INTEGER NOT NULL DEFAULT 0;
ALTER TABLE user_context_derivation_runs ADD COLUMN guardrail_suppressed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE user_context_derivation_runs ADD COLUMN below_formation_bar INTEGER NOT NULL DEFAULT 0;
ALTER TABLE user_context_derivation_runs ADD COLUMN resurface_blocked INTEGER NOT NULL DEFAULT 0;
