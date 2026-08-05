// Pure scope-chip helpers for the Quick Look chips row (frame 08: All ·
// Screen · Audio · Today · This week). Chips are sugar over the SAME operator
// tokens the typed filter syntax uses — the operator text lives in the raw
// query string, so toggling a chip rewrites tokens and lets the ordinary
// search effect rerun. Plain TS, bun-testable.
import { tokenizeQuery } from "./query-tokens";

export type SourceScope = "all" | "screen" | "audio" | "custom";
export type DateScope = "any" | "today" | "week" | "custom";

const AUDIO_VALUES = new Set(["mic", "microphone", "system", "system_audio"]);
const WEEK_TOKEN = "after:7d";
const TODAY_TOKEN = "date:today";

function isSourceToken(token: string): boolean {
  return token.toLowerCase().startsWith("source:");
}

function isDateToken(token: string): boolean {
  return /^(date|after|before):/i.test(token);
}

// The source scope the query's tokens currently express. "custom" = source
// tokens are present but not one of the two chip shapes (e.g. only
// `source:mic`) — the typed chip row surfaces those separately.
export function sourceScopeOf(query: string): SourceScope {
  const values = tokenizeQuery(query)
    .filter(isSourceToken)
    .map((t) => t.slice("source:".length).toLowerCase());
  if (values.length === 0) return "all";
  if (values.length === 1 && values[0] === "screen") return "screen";
  if (values.length > 0 && values.every((v) => AUDIO_VALUES.has(v))) {
    // The Audio chip writes both audio sources; any all-audio combination
    // still reads as the Audio scope (mic+system covers both).
    return "audio";
  }
  return "custom";
}

export function dateScopeOf(query: string): DateScope {
  const tokens = tokenizeQuery(query).filter(isDateToken);
  if (tokens.length === 0) return "any";
  if (tokens.length === 1 && tokens[0].toLowerCase() === TODAY_TOKEN) {
    return "today";
  }
  if (tokens.length === 1 && tokens[0].toLowerCase() === WEEK_TOKEN) {
    return "week";
  }
  return "custom";
}

// Rebuild the query with `family` tokens stripped and `append` tokens added.
// A trailing space keeps the appended operator out of the Filter Value List's
// trailing-partial detection (a committed token is not a partial).
function rewrite(
  query: string,
  drop: (token: string) => boolean,
  append: string[],
): string {
  const kept = tokenizeQuery(query).filter((t) => !drop(t));
  const next = [...kept, ...append].join(" ");
  return append.length > 0 ? `${next} ` : next;
}

export function setSourceScope(
  query: string,
  scope: "all" | "screen" | "audio",
): string {
  const append =
    scope === "screen"
      ? ["source:screen"]
      : scope === "audio"
        ? ["source:mic", "source:system"]
        : [];
  return rewrite(query, isSourceToken, append);
}

export function setDateScope(
  query: string,
  scope: "any" | "today" | "week",
): string {
  const append =
    scope === "today" ? [TODAY_TOKEN] : scope === "week" ? [WEEK_TOKEN] : [];
  return rewrite(query, isDateToken, append);
}
