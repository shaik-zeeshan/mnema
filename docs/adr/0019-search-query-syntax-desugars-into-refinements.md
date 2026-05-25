---
status: accepted
---

# Search query syntax desugars into visible refinements

Mnema adds an opt-in **Search Query Syntax** to the search input: **Body Match Operator** tokens (quoted phrase, `-term` exclusion, `OR`, `term*` prefix) shape text matching, and **Field Operator** tokens (`app:`, `after:`, `before:`, `date:`, `source:`) scope results. Plain text stays the default and an operator-free query behaves exactly as before. This revisits [ADR 0010](0010-search-refinements-and-contextual-entry-points.md), which deliberately added refinements "without creating separate result surfaces or query syntax modes."

It preserves ADR 0010's actual principle — scope must be **explicit and reversible**, and hidden scope "makes missing results look like search failure" — by routing every **Field Operator** through the existing visible, removable **Search Refinement** chips. Typing `app:Safari` lifts the token out of the box and into a chip; it never persists as hidden inline scope. So syntax is a second input path to the same refinement model, not a parallel hidden filter language.

## Considered Options

- **Hidden inline syntax** (GitHub/Gmail style, operators stay as raw text, no chip): rejected — reintroduces exactly the "missing results look like failure" mode ADR 0010 was written to avoid (a typo'd `app:Safri` silently returns nothing).
- **Forgiving fallback** (malformed/unknown operators degrade to literal text): rejected in favor of **strict validation** — a malformed known **Field Operator** value or **Body Match Operator** surfaces an inline, span-highlighted parse error instead of running a misleading or empty search.
- **Any `key:` is a field operator**: rejected — only the known keys are operators, so `http://…`, `error:404`, and `fix:` stay literal body text and the URL/code/error searches that [ADR 0007](0007-search-v1-text-search-in-app-db.md) exists to serve keep working.

## Consequences

- Parsing is **backend-canonical** (app-infra owns recognition, validation, field→refinement extraction, and body→FTS5 translation); the search response returns the residual body query, extracted refinements, and parse errors with spans. A frontend pre-parse is a display-only optimization for optimistic chips and must reconcile to the response.
- `app:` and `source:` refinements become **multi-valued** (sets with OR semantics) rather than single values; `date:` plus `after:`/`before:` write a single date range slot (last-write-wins per bound, both bounds inclusive at day granularity, frozen to concrete timestamps at parse time). Because no search refinements are persisted (saved searches are deferred) and the broker builds them in-process, the single-to-multi change is a clean in-tree replace (`app`→`apps`, `audio_source`→`audio_sources`) rather than a back-compatible dual field; the broker call site is a compile-only update and CLI operator exposure is out of scope.
- Validation problems are returned in-band on the search response as `parse_errors` (machine `kind` + user message + character-offset span + echoed token); a failed search call is reserved for system errors. This moves today's app+source / bad-date errors off the throw path.
- `source:` spans all three capture streams: `source:mic` and `source:system` are **audio-only** (the multi-valued `audio_sources` set), while `source:screen` is **frame-only** (the boolean `screen_source` flag, frame-side counterpart of `audio_sources`). Conflicts are about frame-side vs audio-side scope, surfaced as strict in-band conflict errors: an audio `source:` cannot be combined with `app:` (`app_source_conflict`) or with `source:screen` (`screen_audio_source_conflict`). `source:screen` and `app:` combine freely since both narrow the frame side.
- Result type can be selected either by the existing UI tabs or implicitly by a `source:` value: an audio `source:` narrows to audio, `source:screen` narrows to frames. There is still no dedicated `type:`/`in:` operator — result-type selection rides on `source:` rather than a separate operator.
- A two-tier **Search Operator Suggestion** autocomplete makes operators discoverable (operator names, then values); selecting a value commits the refinement chip and clears its text, converging the dropdown with typed desugaring. `app:` values come from a new query over distinct retained captured apps (not installed apps) so suggestions are result-bearing; date operators suggest preset/relative tokens rather than a date-time picker.
