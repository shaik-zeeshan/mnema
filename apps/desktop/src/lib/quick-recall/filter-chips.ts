// Pure filter-chip helpers for Quick Recall's search mode (extracted from the
// quick-recall +page.svelte in the search-mode extraction slice — no behavior
// change). Chips are RENDERED from the backend desugar (appliedRefinements),
// but the operator TEXT lives in the raw `query` string; these helpers derive
// the chip model and strip a chip's operator token(s) back out of the query.
import type {
  SearchAppRefinement,
  SearchDateRangeRefinement,
  SearchCaptureRefinements,
  SearchParseError,
  AudioSegmentSourceKind,
} from "$lib/types/app-infra";
import {
  appOperatorToken,
  parseToolDate,
  isSameCalendarDay,
  shortDate,
} from "./query-tokens";
import { PART_HOURS, type Pill } from "./natural-tokens";

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

// ---------------------------------------------------------------------------
// Field pills (natural-tokens.ts) → the real operator query
// ---------------------------------------------------------------------------

const SOURCE_PILL: Record<string, [label: string, text: string]> = {
  mic: ["On a call", "on a call"],
  system: ["System audio", "in a video"],
  screen: ["Screen", "on screen"],
};

const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

// The operators one pill stands for, e.g. `app:Arc`, `date:yesterday hours:12-18`.
export function pillOperators(pill: Pill): string {
  switch (pill.kind) {
    case "app":
      return appOperatorToken(pill.app);
    case "source":
      return `source:${pill.source}`;
    case "date":
      return [
        pill.value && `date:${pill.value}`,
        pill.part && `hours:${PART_HOURS[pill.part].join("-")}`,
      ]
        .filter(Boolean)
        .join(" ");
  }
}

// What the pill reads as: `Arc`, `Yesterday · afternoon`, `Afternoon`, `On a call`.
export function pillLabel(pill: Pill): string {
  switch (pill.kind) {
    case "app":
      return pill.app;
    case "source":
      return SOURCE_PILL[pill.source][0];
    case "date":
      if (pill.value === null) return capitalize(pill.part ?? "");
      return pill.part ? `${pill.label} · ${pill.part}` : pill.label;
  }
}

// "Edit as text": the phrase the pill came from, typed back into the field.
export function pillEditText(pill: Pill): string {
  switch (pill.kind) {
    case "app":
      return `in ${pill.app}`;
    case "source":
      return SOURCE_PILL[pill.source][1];
    case "date": {
      const day =
        pill.value === null
          ? null
          : /^\d/.test(pill.value)
            ? `date:${pill.value}`
            : pill.value.replace("-", " ");
      return [day, pill.part].filter(Boolean).join(" ");
    }
  }
}

// The query the backend runs and the footer shows: free text, then each
// pill's operators (`pricing page app:Arc date:yesterday`).
export function operatorQuery(text: string, pills: readonly Pill[]): string {
  return [text.trim(), ...pills.map(pillOperators)].filter(Boolean).join(" ");
}
