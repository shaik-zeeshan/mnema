ALTER TABLE search_documents ADD COLUMN app_bundle_id TEXT;
ALTER TABLE search_documents ADD COLUMN app_name_search_key TEXT;

CREATE INDEX IF NOT EXISTS search_documents_frame_app_identity_idx
    ON search_documents (anchor_type, app_bundle_id, app_name, absolute_start_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS search_documents_frame_app_name_search_key_idx
    ON search_documents (anchor_type, app_name_search_key, app_bundle_id, absolute_start_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS search_documents_frame_bundle_id_refinement_idx
    ON search_documents (
        anchor_type,
        LOWER(TRIM(COALESCE(app_bundle_id, ''))),
        absolute_start_at DESC,
        id DESC
    );
