-- licensegate migration (2026-07-16): the keychain is now the sole source of
-- truth for every licensing artifact (key, receipt, first-seen stamp), so the
-- single-row licensing_state cache shrinks to the one value that must survive
-- in the DB: the anti-rollback high-water mark. Drop/recreate (0047 must NOT
-- be edited in place — field users have it applied; checksum panic), carrying
-- over ONLY max_timestamp_ever_seen_ms.
CREATE TABLE licensing_state_new (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    max_timestamp_ever_seen_ms INTEGER NOT NULL DEFAULT 0  -- anti-rollback high-water mark
);
INSERT INTO licensing_state_new (id, max_timestamp_ever_seen_ms)
    SELECT 1, max_timestamp_ever_seen_ms FROM licensing_state WHERE id = 1;
DROP TABLE licensing_state;
ALTER TABLE licensing_state_new RENAME TO licensing_state;
INSERT OR IGNORE INTO licensing_state (id) VALUES (1);
