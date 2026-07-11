// Pure filter-chip helpers for Quick Recall's search mode (extracted from the
// quick-recall +page.svelte in the search-mode extraction slice — no behavior
// change). Chips are RENDERED from the backend desugar (appliedRefinements),
// but the operator TEXT lives in the raw `query` string; these helpers derive
// the chip model, strip a chip's operator token(s) back out of the query, and
// rebuild canonical operator/plain-language forms for the Ask AI pivot.
import type {
  SearchAppRefinement,
  SearchDateRangeRefinement,
  SearchCaptureRefinements,
  SearchParseError,
  AudioSegmentSourceKind,
} from "$lib/types/app-infra";
import {
  tokenizeQuery,
  quoteOperatorValue,
  toOperatorDay,
  parseToolDate,
  isSameCalendarDay,
  shortDate,
} from "./query-tokens";

// One normalized active filter chip. `kind` groups by operator family; `data`
// is the discriminated source payload used to rebuild syntax
// (e.g. `app:Safari`, `source:microphone`, `after:…`/`before:…`).
export type ActiveFilterChip =
  | {
      id: string;
      kind: "app";
      label: string;
      // The app refinement as the backend desugared it (bundle_id/app_name/any
      // + raw value + human displayName).
      data: SearchAppRefinement;
    }
  | {
      id: string;
      kind: "source";
      label: string;
      // "screen" when source:screen, else the audio source kind.
      data: { source: "screen" } | { source: AudioSegmentSourceKind };
    }
  | {
      id: string;
      kind: "date";
      label: string;
      // The desugared range (ISO start/end + optional origin).
      data: SearchDateRangeRefinement;
    };

// Plain-language label for an audio source kind. Mirrors the spoken phrasing
// the existing answer-source strip uses ("Microphone audio" / "System audio").
export function audioSourceLabel(kind: AudioSegmentSourceKind): string {
  return kind === "microphone" ? "Microphone audio" : "System audio";
}

// Parse a backend date string ("YYYY-MM-DD HH:MM:SS" or ISO) into a Date,
// reusing the same space→T normalization as the Ask AI tool-activity helpers.
// We deliberately do NOT re-validate dates here — the backend already parsed
// the operator; this only formats what it returned. Returns null if unparseable.
export function parseRefinementDate(value: string): Date | null {
  return parseToolDate(value);
}

// Plain-language label for a desugared date range, e.g. "May 1 – May 30",
// "May 1" (single day), or "since May 1" / "until May 30" when one bound is
// open-ended-ish (start === end is treated as a single day). Falls back to the
// raw strings if either bound won't parse, so a chip never renders blank.
export function dateRangeLabel(range: SearchDateRangeRefinement): string {
  const start = parseRefinementDate(range.startAt);
  const end = parseRefinementDate(range.endAt);
  if (start && end) {
    if (isSameCalendarDay(start, end)) {
      return shortDate(start);
    }
    return `${shortDate(start)} – ${shortDate(end)}`;
  }
  if (start) {
    return shortDate(start);
  }
  if (end) {
    return shortDate(end);
  }
  return `${range.startAt} – ${range.endAt}`;
}

// The normalized active-chip list. Order is stable: date first (broadest
// scope), then apps, then sources — so the row reads "when · where · what
// kind". `screenSource` and `audioSources` are mutually exclusive per the
// backend contract, so at most one yields source chips.
export function deriveActiveFilterChips(
  refinements: SearchCaptureRefinements | null,
): ActiveFilterChip[] {
  if (refinements === null) {
    return [];
  }
  const chips: ActiveFilterChip[] = [];

  if (refinements.dateRange) {
    chips.push({
      id: "date",
      kind: "date",
      label: dateRangeLabel(refinements.dateRange),
      data: refinements.dateRange,
    });
  }

  for (const app of refinements.apps ?? []) {
    chips.push({
      id: `app:${app.kind}:${app.value}`,
      kind: "app",
      label: app.displayName,
      data: app,
    });
  }

  if (refinements.screenSource === true) {
    chips.push({
      id: "source:screen",
      kind: "source",
      label: "Screen",
      data: { source: "screen" },
    });
  }
  for (const source of refinements.audioSources ?? []) {
    chips.push({
      id: `source:${source}`,
      kind: "source",
      label: audioSourceLabel(source),
      data: { source },
    });
  }

  return chips;
}

// The operator prefixes a chip of each kind owns. A token (whitespace-delimited
// run) is stripped when its lowercased form starts with any of these.
export function operatorPrefixesForChip(chip: ActiveFilterChip): string[] {
  switch (chip.kind) {
    case "date":
      return ["date:", "after:", "before:"];
    case "app":
      return ["app:"];
    case "source":
      return ["source:"];
  }
}

// The operator values (lowercased, unquoted) that identify THIS chip's token,
// or null for a date chip — which owns the whole `date:`/`after:`/`before:`
// range and so removes its family wholesale. Source chips accept their short
// and long spellings so a `source:mic` token still matches a microphone chip.
export function chipTokenValues(chip: ActiveFilterChip): string[] | null {
  switch (chip.kind) {
    case "app":
      return [chip.data.value.toLowerCase()];
    case "source":
      if (chip.data.source === "screen") return ["screen"];
      return chip.data.source === "microphone"
        ? ["microphone", "mic"]
        : ["system", "system_audio"];
    case "date":
      return null;
  }
}

// Whether `token` is an operator token of one of `prefixes` whose unquoted,
// lowercased value is one of `values`.
export function tokenMatchesChipValue(
  token: string,
  prefixes: string[],
  values: string[],
): boolean {
  const lower = token.toLowerCase();
  const prefix = prefixes.find((p) => lower.startsWith(p));
  if (prefix === undefined) {
    return false;
  }
  const unquoted = token.slice(prefix.length).replace(/^"(.*)"$/, "$1").toLowerCase();
  return values.includes(unquoted);
}

// Remove a single chip's operator token(s) from `raw`, quote-aware. Prefers a
// TARGETED removal — drop only the token whose value matches this chip, so
// removing one `app:`/`source:` chip leaves any sibling chips of the same kind
// intact. The query may carry the user's own spelling, which can differ from
// the backend's desugared value, so when no token matches we fall back to
// dropping every token of that operator family (the original defensive
// behavior) rather than leaving the chip un-removable. Either path is
// quote-aware, so `app:"Google Chrome"` is removed cleanly instead of leaving
// a dangling `Chrome"`. Pure: used by the store's removeChip.
export function stripChipTokens(raw: string, chip: ActiveFilterChip): string {
  const tokens = tokenizeQuery(raw);
  const prefixes = operatorPrefixesForChip(chip);

  const values = chipTokenValues(chip);
  if (values !== null) {
    const index = tokens.findIndex((token) =>
      tokenMatchesChipValue(token, prefixes, values),
    );
    if (index >= 0) {
      tokens.splice(index, 1);
      return tokens.join(" ").trim();
    }
  }

  // Fallback: drop every token of this operator family (spelling-tolerant).
  const kept = tokens.filter((token) => {
    const lower = token.toLowerCase();
    return !prefixes.some((prefix) => lower.startsWith(prefix));
  });
  return kept.join(" ").trim();
}

// ---------------------------------------------------------------------------
// Ask AI pivot scope inheritance (pure builders)
//
// Pivoting search → ask carries the active chip scope into the ask TWO ways:
//
//   1. Structurally, into the SEED. `ask_ai_start`'s `seedQuery` flows to the
//      Rust broker search, which runs the SAME backend `parse_search_query`,
//      so an operator-bearing seed is re-parsed and the seed context is scoped
//      to the chips with no Rust change. We rebuild a CANONICAL operator string
//      from the chips (the desugared truth) + residual rather than forwarding
//      the raw typed query, so a messy/abbreviated raw query still yields a
//      clean, parser-exact seed.
//
//   2. In natural language, into the QUESTION. The question's free-text base is
//      the residual (operators stripped) plus a spoken scope suffix ("in Safari
//      from May 1 to May 30") so the scope is legible to user and agent alike.
// ---------------------------------------------------------------------------

// The canonical operator token(s) one chip contributes to a reconstructed seed.
// Mirrors the parser spellings: `app:<value>` (quoted as needed),
// `source:screen`/`source:microphone`/`source:system_audio` (the audio kind is
// already a parser-accepted word), and `after:<day> before:<day>` for a range
// (or a single `after:<day>`/`before:<day>` when only one bound parses).
export function operatorTokensForChip(chip: ActiveFilterChip): string {
  switch (chip.kind) {
    case "app":
      return `app:${quoteOperatorValue(chip.data.value)}`;
    case "source":
      return `source:${chip.data.source}`;
    case "date": {
      const start = parseRefinementDate(chip.data.startAt);
      const end = parseRefinementDate(chip.data.endAt);
      const parts: string[] = [];
      if (start) parts.push(`after:${toOperatorDay(start)}`);
      if (end) parts.push(`before:${toOperatorDay(end)}`);
      // If neither bound parses we emit nothing (the chip's structural scope is
      // unrecoverable as an operator); the natural-language suffix still carries it.
      return parts.join(" ");
    }
  }
}

// Build a parser-exact seed query from the active chips + residual free text,
// e.g. chips `{app:Safari, date 5/1–5/30}` + residual `deploy error` →
// `app:Safari after:2026-05-01 before:2026-05-30 deploy error`. With no chips
// this is just the residual, so the seed is unchanged from today.
export function buildScopedSeedQuery(
  chips: ActiveFilterChip[],
  residual: string,
): string {
  const operatorTokens = chips
    .map((chip) => operatorTokensForChip(chip))
    .filter((token) => token.length > 0);
  const residualText = residual.trim();
  const parts = [...operatorTokens];
  if (residualText.length > 0) {
    parts.push(residualText);
  }
  return parts.join(" ").trim();
}

// The plain-language scope suffix for one chip: `in Safari`,
// `in microphone audio` / `in system audio`, `in screen captures`, or a date
// window phrased like the existing labels (`from May 1 to May 30`, `on May 1`).
export function scopeSuffixForChip(chip: ActiveFilterChip): string {
  switch (chip.kind) {
    case "app":
      return `in ${chip.data.displayName}`;
    case "source":
      if (chip.data.source === "screen") return "in screen captures";
      return chip.data.source === "microphone"
        ? "in microphone audio"
        : "in system audio";
    case "date": {
      const start = parseRefinementDate(chip.data.startAt);
      const end = parseRefinementDate(chip.data.endAt);
      if (start && end) {
        if (isSameCalendarDay(start, end)) {
          return `on ${shortDate(start)}`;
        }
        return `from ${shortDate(start)} to ${shortDate(end)}`;
      }
      if (start) return `since ${shortDate(start)}`;
      if (end) return `until ${shortDate(end)}`;
      // Unparseable bounds: fall back to the chip's already-formatted label.
      return chip.label;
    }
  }
}

// Build the natural-language question from the residual free text + chips, e.g.
// residual `deploy error` + chips `{app:Safari, date 5/1–5/30}` →
// `deploy error in Safari from May 1 to May 30`. When the residual is empty
// (the query was only operators) we lead with a neutral "Show me everything"
// so the suffix reads as a sentence ("Show me everything in Safari") rather
// than a bare fragment. With no chips this is just the residual (unchanged).
export function buildScopedQuestion(
  residual: string,
  chips: ActiveFilterChip[],
): string {
  const suffixes = chips
    .map((chip) => scopeSuffixForChip(chip))
    .filter((suffix) => suffix.length > 0);
  const residualText = residual.trim();
  if (suffixes.length === 0) {
    return residualText;
  }
  const base = residualText.length > 0 ? residualText : "Show me everything";
  return [base, ...suffixes].join(" ").trim();
}

// ---------------------------------------------------------------------------
// Friendly parse-error message (pure helper)
//
// The backend parse messages are accurate but terse/technical ("…must be a
// valid RFC3339 timestamp", "windowTitle must be non-empty", "OR needs a
// search term on both sides"). This maps the known `kind` values to plain
// language, interpolating the offending `token` where it sharpens the hint
// (e.g. `"notadate" isn't a date I understand`). Any unmapped kind falls back
// to the raw backend `message`, so a new backend error never renders blank.
// Only the FIRST parse error is ever shown, so this only formats one.
// ---------------------------------------------------------------------------
export function friendlyParseError(err: SearchParseError): string {
  const token = err.token.trim();
  switch (err.kind) {
    case "bad_date":
      return token.length > 0
        ? `“${token}” isn't a date I understand. Try a day like 2024-05-01, or today / yesterday.`
        : "That date filter isn't one I understand. Try a day like 2024-05-01, or today / yesterday.";
    case "unknown_source":
      return token.length > 0
        ? `“${token}” isn't a source I know. Use source:mic, source:system, or source:screen.`
        : "Use source:mic, source:system, or source:screen.";
    case "unbalanced_quote":
      return "There's an unclosed quote in your search — add the matching closing quote.";
    case "empty_value":
      return "That filter is missing a value — add a name after the colon.";
    case "app_source_conflict":
      return "app: and source: can't be combined — app: narrows the screen, source: narrows audio.";
    case "screen_audio_source_conflict":
      return "source:screen can't be combined with source:mic or source:system.";
    case "dangling_or":
      return "OR needs a search term on both sides.";
    case "pure_negation":
      return "An exclusion like -term needs at least one positive term to match.";
    default:
      // Unknown backend kind: surface its message rather than render blank.
      return err.message;
  }
}
