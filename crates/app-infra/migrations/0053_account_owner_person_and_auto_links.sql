-- Voice enrollment storage + owner-only auto-linking.
--
-- 1. `person_profiles.is_account_owner` marks the ONE Person Profile that is the
--    account owner — the person who recorded a deliberate enrollment clip. The
--    partial unique index enforces "at most one owner" in the schema, so no
--    write path can produce a second one even by accident. (SQLite indexes only
--    the rows matching the WHERE clause, so the unlimited `0` rows are not
--    constrained.)
--
-- 2. `recording_speaker_clusters.person_link_auto` records HOW a cluster came to
--    be linked to a person: 0 = a human named or confirmed it, 1 = owner-only
--    high-confidence auto-linking decided it. The user has to be able to see
--    what was decided for them and undo it, which needs the distinction in the
--    data, not just in the UI.
ALTER TABLE person_profiles ADD COLUMN is_account_owner INTEGER NOT NULL DEFAULT 0;

CREATE UNIQUE INDEX IF NOT EXISTS idx_person_profiles_account_owner
    ON person_profiles(is_account_owner) WHERE is_account_owner = 1;

ALTER TABLE recording_speaker_clusters ADD COLUMN person_link_auto INTEGER NOT NULL DEFAULT 0;
