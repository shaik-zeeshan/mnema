-- Supersede wrong Conclusions (ADR 0046). A Conclusion whose statement turns out
-- to be wrong is RETIRED, not revised in place: the corrected belief forms as a
-- new row at formation confidence, and the wrong row keeps its history but leaves
-- every read surface (status 'superseded'), linked to its successor.
--
-- `status` has no CHECK constraint, so 'superseded' is just a new string value —
-- no table rewrite. `superseded_by` points a retired row at the successor that
-- replaced it (the reinforced-or-newly-formed citing belief); NULL for live rows.
-- Matches the existing FK-less column style on this table (cf. 0025's `pinned`).
ALTER TABLE user_context_conclusions ADD COLUMN superseded_by INTEGER;

-- Dismissal provenance: a supersede writes a `user_context_dismissals` row so the
-- resurface gate can block re-forming the retired statement from the same
-- evidence. `source = 'supersede'` marks it a machine correction (resurface target
-- is the retained row); user Dismiss rows are `source = 'user'`.
ALTER TABLE user_context_dismissals ADD COLUMN source TEXT NOT NULL DEFAULT 'user';

-- Supersede observability on the derivation-run ledger (kind 'conclusion' rows),
-- mirroring the 0032 gate-drop counters:
--   superseded         -- wrong rows retired downward this pass
--   supersede_degraded -- stronger rows dropped one contradiction step, not retired
--   supersede_blocked  -- supersedes_id ignored (pinned / missing / self / foreign)
ALTER TABLE user_context_derivation_runs ADD COLUMN superseded INTEGER NOT NULL DEFAULT 0;
ALTER TABLE user_context_derivation_runs ADD COLUMN supersede_degraded INTEGER NOT NULL DEFAULT 0;
ALTER TABLE user_context_derivation_runs ADD COLUMN supersede_blocked INTEGER NOT NULL DEFAULT 0;
