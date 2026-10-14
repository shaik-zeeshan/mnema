// Quick Recall's empty, partial and recent states (recall.html: recent, no
// results, indexing behind, meaning off / building, audio pending, not
// recording): the pure rules behind RecentRows / EmptyStates and the pill
// popover's counts. The store makes the calls; this decides what they say.
import type {
  QuickRecallIndexStatus,
  RecentQuery,
  SearchFacet,
  SearchFacetCounts,
} from "$lib/types/app-infra";
import { operatorQuery, pillEditText, pillLabel } from "./filter-chips";
import type { PillOption } from "./filterSurfaces.svelte";
import type { Pill } from "./natural-tokens";

// ── Recent queries ──────────────────────────────────────────────────────────

export const RECENT_MAX = 8;

// The text a query is remembered as: the typed words plus each pill back as
// its phrase, so picking it re-tints and re-pills the same way.
export function recentText(residual: string, pills: readonly Pill[]): string {
  return [residual.trim(), ...pills.map(pillEditText)].filter(Boolean).join(" ");
}

// Newest first, one entry per text (case-insensitive), at most RECENT_MAX.
export function withRecent(list: readonly RecentQuery[], entry: RecentQuery): RecentQuery[] {
  const key = entry.text.trim().toLowerCase();
  if (!key) return [...list];
  return [entry, ...list.filter((q) => q.text.trim().toLowerCase() !== key)].slice(0, RECENT_MAX);
}

// "just now" · "12m ago" · "3h ago" · "yesterday" · "Wed" · "Oct 3".
export function agoLabel(at: number, now: Date): string {
  const mins = Math.floor((now.getTime() - at) / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const then = new Date(at);
  const days = Math.round(
    (new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() -
      new Date(then.getFullYear(), then.getMonth(), then.getDate()).getTime()) /
      86_400_000,
  );
  if (days === 0) return `${Math.floor(mins / 60)}h ago`;
  if (days === 1) return "yesterday";
  if (days < 7) return then.toLocaleDateString("en-US", { weekday: "short" });
  return then.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

// ── The one notice line over results / in no-results ────────────────────────

// Unread frames only count as "behind" once the oldest has waited this long —
// a few frames are always in flight while recording.
export const INDEX_LAG_MS = 60_000;
// `semanticBuilding` stays true near 100% (new captures briefly lack vectors),
// so the building line only shows below this.
export const SEMANTIC_BUILDING_BELOW = 95;

export type SearchFlags = { semanticDegraded: boolean; untranscribed: number };
export type Notice = {
  tone: "info" | "warn" | null;
  icon: "info" | "off" | "alert";
  text: string;
  settings?: boolean;
};

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

// At most one line, most actionable first.
export function searchNotice(
  status: QuickRecallIndexStatus | null,
  flags: SearchFlags | null,
  recording: boolean | null,
  now: number,
): Notice | null {
  if (flags?.semanticDegraded) {
    return {
      tone: "warn",
      icon: "alert",
      text: "Search by meaning hit an error — these are keyword matches only.",
    };
  }
  const lag = status?.oldestQueuedMs != null ? now - status.oldestQueuedMs : 0;
  if (status && status.queuedFrames > 0 && lag >= INDEX_LAG_MS) {
    return {
      tone: "info",
      icon: "info",
      text: `${indexingLine(status, now)} Newer moments may be missing.`,
    };
  }
  if (flags && flags.untranscribed > 0) {
    return {
      tone: "info",
      icon: "info",
      text: `${plural(flags.untranscribed, "recording")} ${flags.untranscribed === 1 ? "is" : "are"} still being transcribed — ${flags.untranscribed === 1 ? "it’ll show up once it is" : "they’ll show up once they are"}.`,
    };
  }
  const pct = status?.semanticIndexedPercent;
  if (status?.semanticBuilding && pct != null && pct < SEMANTIC_BUILDING_BELOW) {
    return {
      tone: "info",
      icon: "info",
      text: `Search by meaning is still building — ${pct}% of your history so far. Keyword results are complete.`,
    };
  }
  if (recording === false) {
    return {
      tone: null,
      icon: "off",
      text: "Mnema isn’t recording right now — this searches everything captured before it stopped.",
    };
  }
  if (status && pct === null) {
    return {
      tone: null,
      icon: "off",
      text: "Searching keywords only — search by meaning is off.",
      settings: true,
    };
  }
  return null;
}

// "Still reading your last 4 min — 38 frames queued."
export function indexingLine(status: QuickRecallIndexStatus, now: number): string {
  const mins = Math.max(1, Math.round((now - (status.oldestQueuedMs ?? now)) / 60_000));
  const span = mins < 90 ? `${mins} min` : plural(Math.round(mins / 60), "hour");
  return `Still reading your last ${span} — ${plural(status.queuedFrames, "frame")} queued.`;
}

// ── Facet counts ────────────────────────────────────────────────────────────

export type FacetCall = { facet: SearchFacet; query: string };
export type FacetResult = FacetCall & { counts: SearchFacetCounts };
export type CountLabel = number | string;

// "50+" when the backend's per-kind cap was hit: the count is a lower bound.
export const countLabel = (n: number, capped: boolean): CountLabel => (capped ? `${n}+` : n);

const without = (pills: readonly Pill[], kind: Pill["kind"]) => pills.filter((p) => p.kind !== kind);

// The count calls one pill's popover needs. A date pill takes two: parts of
// the same day (hours swapped out), and other days (the whole pill gone).
export function popoverCalls(residual: string, pills: readonly Pill[], pill: Pill): FacetCall[] {
  const query = operatorQuery(residual, pills);
  if (pill.kind === "app") return [{ facet: "app", query }];
  if (pill.kind === "source") return [{ facet: "source", query }];
  return [
    { facet: "partOfDay", query },
    { facet: "day", query: operatorQuery(residual, without(pills, "date")) },
  ];
}

const SOURCE_KEY = { screen: "screen", mic: "microphone", system: "system_audio" } as const;

// Counts keyed by `PillOption.op` ("" = the "Any …" row) — `search.facetCounts`.
export function popoverCounts(
  pill: Pill,
  options: readonly PillOption[],
  results: readonly FacetResult[],
  now: Date,
): Record<string, CountLabel> {
  const out: Record<string, CountLabel> = {};
  const of = (facet: SearchFacet) => results.find((r) => r.facet === facet)?.counts;
  const bucket = (c: SearchFacetCounts, match: (key: string, label: string) => boolean) =>
    c.buckets.filter((b) => match(b.key, b.label)).reduce((n, b) => n + b.count, 0);
  for (const option of options) {
    const next = option.pill;
    let c: SearchFacetCounts | undefined;
    let n: number | undefined;
    if (pill.kind === "app" || pill.kind === "source") {
      c = of(pill.kind);
      if (c && next?.kind === "app") {
        const name = next.app.toLowerCase();
        n = bucket(c, (k, l) => l.toLowerCase() === name || k.toLowerCase() === name);
      } else if (c && next?.kind === "source") {
        n = bucket(c, (k) => k === SOURCE_KEY[next.source]);
      } else if (c) n = c.total;
    } else if (next === null) {
      c = of("day");
      n = c?.total;
    } else if (next.kind === "date" && next.value === pill.value && pill.value !== "this-week" && pill.value !== "last-week") {
      // Same day, another part of it (or all of it).
      c = of("partOfDay");
      if (c) n = next.part ? bucket(c, (k) => k === next.part) : c.total;
    } else if (next.kind === "date" && (next.value === "this-week" || next.value === "last-week")) {
      c = of("day");
      const days = weekDays(now, next.value === "this-week" ? 0 : -1);
      if (c) n = bucket(c, (k) => days.has(k));
    }
    // A capped zero ("0+") says nothing; leave it unnumbered.
    if (c && n !== undefined && !(c.capped && n === 0)) out[option.op] = countLabel(n, c.capped);
  }
  return out;
}

const pad = (n: number) => String(n).padStart(2, "0");
const isoDay = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

// Monday..Sunday of this week (offset 0) or an earlier one — `search/dates.rs`.
export function weekDays(now: Date, offset: number): Set<string> {
  const monday = now.getDate() - ((now.getDay() + 6) % 7) + offset * 7;
  return new Set(
    Array.from({ length: 7 }, (_, i) => isoDay(new Date(now.getFullYear(), now.getMonth(), monday + i))),
  );
}

// ── No results: ways out ────────────────────────────────────────────────────

export type WayOut =
  | { kind: "pills"; pills: Pill[]; label: string; call: FacetCall }
  | { kind: "ask"; label: string };

// "on Tue Oct 6 afternoon", "yesterday", "this afternoon".
export function datePhrase(pill: Extract<Pill, { kind: "date" }>): string {
  if (pill.value === null) return `in the ${pill.part}`;
  if (pill.value === "today" && pill.part) return `this ${pill.part}`;
  const day = /^(today|yesterday|this-week|last-week|this-month|last-month)$/.test(pill.value)
    ? pill.label.toLowerCase()
    : `on ${pill.label}`;
  return pill.part ? `${day} ${pill.part}` : day;
}

// Each way out is the same search with one filter loosened (recall.html
// `relaxations`): all of the day first, then dropping each pill, then Ask.
// Rows whose count comes back 0 are left out by the caller.
export function waysOut(residual: string, pills: readonly Pill[], canAsk: boolean): WayOut[] {
  const out: WayOut[] = [];
  const date = pills.find((p): p is Extract<Pill, { kind: "date" }> => p.kind === "date");
  if (date?.value && date.part) {
    const wider = pills.map((p) => (p === date ? { ...date, part: null } : p));
    out.push({
      kind: "pills",
      pills: wider,
      label: `All of ${datePhrase({ ...date, part: null }).replace(/^on /, "")}, not just the ${date.part}`,
      call: { facet: "partOfDay", query: operatorQuery(residual, wider) },
    });
  }
  for (const pill of pills) {
    const rest = without(pills, pill.kind);
    out.push({
      kind: "pills",
      pills: rest,
      label: pill.kind === "date" ? `Any time — not just ${datePhrase(pill)}` : `Drop “${pillLabel(pill)}”`,
      call: { facet: pill.kind === "source" ? "source" : pill.kind === "app" ? "app" : "day", query: operatorQuery(residual, rest) },
    });
  }
  if (canAsk && residual.trim()) out.push({ kind: "ask", label: "Ask instead — I’ll look harder" });
  return out;
}
