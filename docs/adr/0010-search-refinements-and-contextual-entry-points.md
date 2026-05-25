# Search refinements and contextual entry points share one result surface

Mnema will improve search UX by adding **Search Refinement** controls and contextual **Search Entry Point** actions without creating separate result surfaces or query syntax modes. Entry points such as **Visible Timeline Search** and **Current App Search** open the normal search modal with visible removable refinements, so scoped search is explicit and reversible.

**Visible Timeline Search** scopes results by the dashboard timeline viewport time range and includes both **Captured Frame** and **Audio Transcription Span** results. **Current App Search** scopes results by the active dashboard **Captured Frame**'s retained app context, uses bundle identifier as canonical identity when available, and remains frame-only until audio results have an explicit **Search Context Alignment** policy. This avoids implying that microphone or system-audio transcript spans natively belong to the app visible on screen.

Refinements are search semantics rather than frontend result decoration: they apply before grouping, representative-anchor selection, and pagination. Date-range refinements freeze to concrete timestamps when selected so an open search modal does not silently change scope as the dashboard timeline or wall clock moves.

## Alternatives Rejected

- Separate scoped search surfaces: duplicates result behavior and makes scoped search harder to reason about.
- Hidden entry-point scope: makes missing results look like search failure.
- Applying app refinement to audio by guessing nearby screen context: conflates native audio source data with derived alignment and can misrepresent what app an audio span came from.

## Amended By

- [ADR 0019](0019-search-query-syntax-desugars-into-refinements.md) revisits "without query syntax modes": opt-in query syntax is now supported, but field operators desugar into the visible, removable refinements defined here, preserving the explicit-and-reversible scope principle.
