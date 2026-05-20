ALTER TABLE search_documents ADD COLUMN app_bundle_id TEXT;

CREATE INDEX IF NOT EXISTS search_documents_frame_app_identity_idx
    ON search_documents (anchor_type, app_bundle_id, app_name, absolute_start_at DESC, id DESC);
