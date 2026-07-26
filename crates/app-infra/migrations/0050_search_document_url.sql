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
