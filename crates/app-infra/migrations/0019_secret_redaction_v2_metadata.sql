ALTER TABLE processing_results
    ADD COLUMN redaction_detector_version TEXT;

ALTER TABLE processing_results
    ADD COLUMN redaction_checked_at TEXT;

ALTER TABLE secret_redactions
    ADD COLUMN surface_kind TEXT NOT NULL DEFAULT 'result_text'
        CHECK (surface_kind IN (
            'result_text',
            'ocr_visual_line',
            'ocr_observation',
            'transcript_segment',
            'transcript_word',
            'context_text'
        ));

ALTER TABLE secret_redactions
    ADD COLUMN redaction_scope TEXT NOT NULL DEFAULT 'exact_span'
        CHECK (redaction_scope IN ('exact_span', 'redaction_unit'));

CREATE INDEX IF NOT EXISTS secret_redactions_result_surface_idx
    ON secret_redactions(processing_result_id, surface_kind, id);
