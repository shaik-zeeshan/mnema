-- ADR 0032: the **Activity Category** taxonomy moves from developer-specific
-- domains (Coding, Research, Communication, Design, Testing, Personal,
-- Distractions) to eight profession-neutral *work modes* (Creating,
-- Communication, Meetings, Research, Learning, Organizing, Personal,
-- Entertainment). Relabel existing rows once so the store layer only ever
-- round-trips the new set: Coding/Testing/Design coarsen into Creating, and
-- Distractions becomes the neutral content label Entertainment (the
-- derailed/focused judgment belongs exclusively to Focus Classification).
-- Research/Communication/Personal carry over unchanged. Both the engine
-- `category` column AND the #108 user-correction `corrected_category` column
-- are relabeled — corrections are preserved but coarsened into the superset.

UPDATE user_context_activities
SET category = 'creating'
WHERE category IN ('coding', 'testing', 'design');

UPDATE user_context_activities
SET category = 'entertainment'
WHERE category = 'distractions';

UPDATE user_context_activities
SET corrected_category = 'creating'
WHERE corrected_category IN ('coding', 'testing', 'design');

UPDATE user_context_activities
SET corrected_category = 'entertainment'
WHERE corrected_category = 'distractions';
