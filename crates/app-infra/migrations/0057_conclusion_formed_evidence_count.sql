-- How many supporting Activities a Conclusion formed from ("Formed from N
-- supporting activities · 62%"). Stamped once, on the row's first insert, while
-- the evidence list is in hand: every later reinforcement deletes and re-inserts
-- the evidence links, so the count can't be rebuilt afterwards. Nullable: rows
-- formed before this column (or by the evidence-less upsert) read NULL.
ALTER TABLE user_context_conclusions ADD COLUMN formed_evidence_count INTEGER;
