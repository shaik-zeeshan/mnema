-- The guarded (host[:port]/path, secret-redacted) browser URL of the frame a
-- search document anchors to. Never the raw URL: the URL refinement filters on
-- exactly the text the broker is willing to emit, so a client cannot use the
-- filter as an oracle for query strings, fragments, or path secrets the read-time
-- guard refuses to hand out. NULL means "not yet projected" (the startup backfill
-- claims it); an empty string means "resolved, no broker-safe url".
--
-- No refinement index: the only predicates over this column are LIKE '%…%' and
-- REGEXP, neither of which can seek an index, so one would be dead weight.
ALTER TABLE search_documents ADD COLUMN url TEXT;

-- Which revision of the guard's redaction rules produced the `url` above
-- (`app_infra::brokered_access::URL_GUARD_VERSION`). The stored copy is made
-- ONCE at projection time while the broker recomputes the guard on every read,
-- so "the filter can only match text the boundary would also emit" holds only
-- while both copies came from the same rules. This column is what lets a later,
-- stricter revision find its stale rows:
--
--   UPDATE search_documents SET url = NULL
--    WHERE url_guard_version < <new value> AND COALESCE(url, '') <> '';
--
-- NULL-ing `url` hands them back to the startup backfill, which re-guards them
-- and stamps the new version. Without this column there is no way to tell which
-- rows predate a rule change, and old rows stay filterable at the looser
-- redaction — the exact oracle ADR 0038 forbids.
ALTER TABLE search_documents ADD COLUMN url_guard_version INTEGER;

-- ...but the BACKFILL's own probe (`WHERE url IS NULL`) is sargable, and it runs
-- on every startup forever. A PARTIAL index over exactly the un-projected rows is
-- free on the hot insert path — the insert binds `url` to '' (never NULL), so a
-- live row never satisfies the index's WHERE and never enters the index — and it
-- turns the drained probe from "visit every frame document" into an empty-index
-- scan. Measured at 300k documents: 4.28s per startup without it, ~0.2ms with it.
CREATE INDEX IF NOT EXISTS search_documents_url_backfill_idx
    ON search_documents (id)
    WHERE url IS NULL;
