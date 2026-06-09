-- User Context (issue #88) user-authored Context (issue #107).
--
-- A standing statement the user wrote about themselves ("I'm a designer", "I
-- care about X"). Unlike a derived Conclusion this is USER-ASSERTED, not
-- inferred from captures, so:
--   * it carries NO confidence and is NEVER subject to the Confidence Policy /
--     decay (it never fades),
--   * Retention Policy aging does NOT touch it, and the Delete Recent Capture
--     cascade leaves it intact (it is not derived from any capture subject — no
--     FK to frames/audio_segments),
--   * it is provided to the Reasoning Engine alongside derived User Context to
--     steer derivation, and
--   * it IS cleared by the explicit Wipe User Context control (`wipe_all`).
--
-- The statement is stored verbatim as the user wrote it. The Sensitive Category
-- Guardrail governs only what the ENGINE may surface, not what is stored here.
--
-- Timestamp convention (same as 0022..0025): INTEGER unix milliseconds columns
-- named `*_at_ms`, set from Rust at insert/update (NOT CURRENT_TIMESTAMP).

CREATE TABLE IF NOT EXISTS user_context_authored (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The standing statement, stored verbatim as the user wrote it.
    text TEXT NOT NULL,
    -- Optional short grouping handle the user can attach (mirrors a Conclusion's
    -- Subject), NULL when unspecified.
    topic TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
