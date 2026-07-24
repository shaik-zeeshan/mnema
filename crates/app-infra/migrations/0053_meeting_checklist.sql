-- Persisted done-state for a meeting recap's action-item checklist.
--
-- The recap markdown (in the trigger-run conversation) owns the item SET; this
-- column stores only WHICH items the user has ticked, as a JSON array of the
-- checked item texts. NULL means nothing ticked. It lives on the meeting row so
-- it is deleted with the meeting (retention / delete), like `notes`.
ALTER TABLE meetings ADD COLUMN checklist_json TEXT;
